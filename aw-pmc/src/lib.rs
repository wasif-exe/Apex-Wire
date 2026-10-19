const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_TYPE_HW_CACHE: u32 = 3;
const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const PERF_COUNT_HW_CACHE_MISSES: u64 = 3;
const PERF_COUNT_HW_BRANCH_MISSES: u64 = 5;
const PERF_COUNT_HW_CACHE_L1D: u64 = 0;
const PERF_COUNT_HW_CACHE_OP_READ: u64 = 0;
const PERF_COUNT_HW_CACHE_RESULT_MISS: u64 = 1;
const PERF_ATTR_SIZE: u32 = 128;

#[cfg(target_arch = "x86_64")]
const SYS_PERF_EVENT_OPEN: libc::c_long = 298;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PmcStatus {
    #[default]
    Fallback,
    PerfAvailable,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PmcSnapshot {
    pub cycles: u64,
    pub instructions: u64,
    pub l1d_misses: u64,
    pub llc_misses: u64,
    pub branch_misses: u64,
    pub status: PmcStatus,
}

impl PmcSnapshot {
    #[inline(always)]
    pub fn ipc(&self) -> f64 {
        if self.cycles == 0 {
            0.0
        } else {
            self.instructions as f64 / self.cycles as f64
        }
    }

    #[inline(always)]
    pub fn l1d_miss_rate(&self) -> f64 {
        if self.instructions == 0 {
            0.0
        } else {
            self.l1d_misses as f64 / self.instructions as f64
        }
    }

    #[inline(always)]
    pub fn llc_miss_rate(&self) -> f64 {
        if self.instructions == 0 {
            0.0
        } else {
            self.llc_misses as f64 / self.instructions as f64
        }
    }
}

struct CounterFd {
    fd: i32,
}

impl CounterFd {
    fn open(type_: u32, config: u64) -> Option<Self> {
        #[cfg(target_arch = "x86_64")]
        {
            let mut attr = [0u8; 128];
            attr[0..4].copy_from_slice(&type_.to_ne_bytes());
            attr[4..8].copy_from_slice(&PERF_ATTR_SIZE.to_ne_bytes());
            attr[8..16].copy_from_slice(&config.to_ne_bytes());
            let flags: u64 = (1 << 0) | (1 << 5);
            attr[40..48].copy_from_slice(&flags.to_ne_bytes());
            // SAFETY: perf_event_open syscall with zeroed attr buffer and valid size field.
            let fd = unsafe {
                libc::syscall(
                    SYS_PERF_EVENT_OPEN,
                    attr.as_ptr() as *const libc::c_void,
                    0i32,
                    -1i32,
                    -1i32,
                    0u64,
                )
            };
            if fd >= 0 {
                Some(Self { fd: fd as i32 })
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (type_, config);
            None
        }
    }

    #[inline(always)]
    fn read(&self) -> u64 {
        let mut val = 0u64;
        // SAFETY: reading 8 bytes from a valid perf_event_open file descriptor.
        unsafe {
            libc::read(
                self.fd,
                &mut val as *mut u64 as *mut libc::c_void,
                8,
            );
        }
        val
    }

    fn enable(&self) {
        // SAFETY: ioctl PERF_EVENT_IOC_ENABLE on valid perf fd.
        unsafe {
            libc::ioctl(self.fd, 0x2400, 0);
        }
    }

    #[allow(dead_code)]
    fn disable(&self) {
        // SAFETY: ioctl PERF_EVENT_IOC_DISABLE on valid perf fd.
        unsafe {
            libc::ioctl(self.fd, 0x2401, 0);
        }
    }
}

impl Drop for CounterFd {
    fn drop(&mut self) {
        // SAFETY: closing perf event file descriptor.
        unsafe {
            libc::close(self.fd);
        }
    }
}

pub struct PmcProfiler {
    cycles: Option<CounterFd>,
    instructions: Option<CounterFd>,
    l1d_misses: Option<CounterFd>,
    llc_misses: Option<CounterFd>,
    branch_misses: Option<CounterFd>,
    status: PmcStatus,
}

impl PmcProfiler {
    pub fn new() -> Self {
        let l1d_config = PERF_COUNT_HW_CACHE_L1D
            | (PERF_COUNT_HW_CACHE_OP_READ << 8)
            | (PERF_COUNT_HW_CACHE_RESULT_MISS << 16);

        let cycles = CounterFd::open(PERF_TYPE_HARDWARE, PERF_COUNT_HW_CPU_CYCLES);
        let instructions = CounterFd::open(PERF_TYPE_HARDWARE, PERF_COUNT_HW_INSTRUCTIONS);
        let l1d_misses = CounterFd::open(PERF_TYPE_HW_CACHE, l1d_config);
        let llc_misses = CounterFd::open(PERF_TYPE_HARDWARE, PERF_COUNT_HW_CACHE_MISSES);
        let branch_misses = CounterFd::open(PERF_TYPE_HARDWARE, PERF_COUNT_HW_BRANCH_MISSES);

        let available = cycles.is_some() && instructions.is_some();
        let status = if available {
            PmcStatus::PerfAvailable
        } else {
            PmcStatus::Fallback
        };

        let prof = Self {
            cycles,
            instructions,
            l1d_misses,
            llc_misses,
            branch_misses,
            status,
        };
        if available {
            prof.enable_all();
        }
        prof
    }

    fn enable_all(&self) {
        if let Some(ref c) = self.cycles { c.enable(); }
        if let Some(ref c) = self.instructions { c.enable(); }
        if let Some(ref c) = self.l1d_misses { c.enable(); }
        if let Some(ref c) = self.llc_misses { c.enable(); }
        if let Some(ref c) = self.branch_misses { c.enable(); }
    }

    #[inline(always)]
    pub fn snapshot(&self) -> PmcSnapshot {
        if self.status == PmcStatus::Fallback {
            return PmcSnapshot {
                status: PmcStatus::Fallback,
                ..Default::default()
            };
        }
        PmcSnapshot {
            cycles: self.cycles.as_ref().map_or(0, |c| c.read()),
            instructions: self.instructions.as_ref().map_or(0, |c| c.read()),
            l1d_misses: self.l1d_misses.as_ref().map_or(0, |c| c.read()),
            llc_misses: self.llc_misses.as_ref().map_or(0, |c| c.read()),
            branch_misses: self.branch_misses.as_ref().map_or(0, |c| c.read()),
            status: self.status,
        }
    }

    #[inline(always)]
    pub fn status(&self) -> PmcStatus {
        self.status
    }

    #[inline(always)]
    pub fn status_label(&self) -> &'static str {
        match self.status {
            PmcStatus::PerfAvailable => "PERF_AVAILABLE",
            PmcStatus::Fallback => "FALLBACK",
        }
    }
}

impl Default for PmcProfiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmc_profiler_constructs() {
        let prof = PmcProfiler::new();
        let snap = prof.snapshot();
        assert!(
            snap.status == PmcStatus::PerfAvailable || snap.status == PmcStatus::Fallback
        );
    }

    #[test]
    fn pmc_snapshot_ipc_zero_safe() {
        let snap = PmcSnapshot::default();
        assert_eq!(snap.ipc(), 0.0);
        assert_eq!(snap.l1d_miss_rate(), 0.0);
        assert_eq!(snap.llc_miss_rate(), 0.0);
    }
}

