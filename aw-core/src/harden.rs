use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

pub fn install_signal_handlers() {
    // SAFETY: signal() with SIGINT/SIGTERM and a simple async-signal-safe handler.
    unsafe {
        libc::signal(libc::SIGINT, shutdown_handler as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, shutdown_handler as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

extern "C" fn shutdown_handler(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Release);
}

pub fn is_shutdown() -> bool {
    SHUTDOWN.load(Ordering::Acquire)
}

pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::Release);
}

pub fn reset_shutdown() {
    SHUTDOWN.store(false, Ordering::Release);
}

pub fn pin_thread(core: usize) {
    // SAFETY: cpu_set_t zeroed then populated with valid core id.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(core, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

pub fn set_realtime_priority(priority: i32) -> bool {
    let param = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: sched_setscheduler with SCHED_FIFO and valid param.
    let ret = unsafe {
        libc::sched_setscheduler(0, libc::SCHED_FIFO, &param)
    };
    ret == 0
}

pub fn lock_memory() -> bool {
    // SAFETY: mlockall with MCL_CURRENT | MCL_FUTURE to prevent page faults.
    let ret = unsafe {
        libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE)
    };
    ret == 0
}

pub fn unlock_memory() {
    // SAFETY: munlockall to release locked pages.
    unsafe {
        libc::munlockall();
    }
}

pub struct Watchdog {
    running: Arc<AtomicBool>,
    heartbeat: Arc<AtomicU64>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Watchdog {
    pub fn start(
        name: &str,
        timeout: Duration,
        on_timeout: impl Fn() + Send + 'static,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let heartbeat = Arc::new(AtomicU64::new(0));
        let r = running.clone();
        let hb = heartbeat.clone();
        let t = timeout;

        let handle = std::thread::Builder::new()
            .name(format!("wd-{}", name))
            .spawn(move || {
                let mut last_tick = hb.load(Ordering::Acquire);
                let mut last_check = Instant::now();

                while r.load(Ordering::Acquire) && !is_shutdown() {
                    std::thread::sleep(t / 2);
                    let now = Instant::now();
                    let current_tick = hb.load(Ordering::Acquire);

                    if current_tick != last_tick {
                        last_tick = current_tick;
                        last_check = now;
                    } else if now.duration_since(last_check) > t {
                        on_timeout();
                        break;
                    }
                }
            })
            .expect("failed to spawn watchdog thread");

        Self {
            running,
            heartbeat,
            handle: Some(handle),
        }
    }

    pub fn tick(&self) {
        self.heartbeat.fetch_add(1, Ordering::Release);
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct ProductionGuard {
    mem_locked: bool,
}

impl ProductionGuard {
    pub fn activate(core: usize, rt_priority: i32) -> Self {
        install_signal_handlers();
        pin_thread(core);
        let _ = set_realtime_priority(rt_priority);
        let mem_locked = lock_memory();
        Self { mem_locked }
    }

    pub fn mem_locked(&self) -> bool {
        self.mem_locked
    }
}

impl Drop for ProductionGuard {
    fn drop(&mut self) {
        if self.mem_locked {
            unlock_memory();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_flag() {
        reset_shutdown();
        assert!(!is_shutdown());
        request_shutdown();
        assert!(is_shutdown());
        reset_shutdown();
        assert!(!is_shutdown());
    }

    #[test]
    fn watchdog_ticks() {
        let triggered = Arc::new(AtomicBool::new(false));
        let t = triggered.clone();
        let mut wd = Watchdog::start(
            "test",
            Duration::from_millis(500),
            move || { t.store(true, Ordering::Release); },
        );

        for _ in 0..10 {
            wd.tick();
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(!triggered.load(Ordering::Acquire));
        wd.stop();
    }

    #[test]
    fn watchdog_timeout() {
        let triggered = Arc::new(AtomicBool::new(false));
        let t = triggered.clone();
        let mut wd = Watchdog::start(
            "test-timeout",
            Duration::from_millis(100),
            move || { t.store(true, Ordering::Release); },
        );

        std::thread::sleep(Duration::from_millis(300));
        assert!(triggered.load(Ordering::Acquire));
        wd.stop();
    }
}
