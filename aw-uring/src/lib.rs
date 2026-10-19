use std::os::unix::io::RawFd;
use std::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use io_uring::{opcode, types, IoUring};
pub struct AsyncReactor {
    ring: Arc<Mutex<IoUring>>,
    wakers: Arc<Mutex<HashMap<RawFd, Waker>>>,
}
impl AsyncReactor {
    pub fn new(entries: u32) -> anyhow::Result<Self> {
        let ring = IoUring::new(entries)?;
        Ok(Self {
            ring: Arc::new(Mutex::new(ring)),
            wakers: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    pub fn register_interest(&self, fd: RawFd, waker: Waker) {
        let mut wakers = self.wakers.lock().unwrap();
        wakers.insert(fd, waker);
        let mut ring = self.ring.lock().unwrap();
        let read_e = opcode::PollAdd::new(types::Fd(fd), libc::POLLIN as u32)
            .build()
            .user_data(fd as u64);
        unsafe {
            ring.submission().push(&read_e).unwrap();
        }
        ring.submit().unwrap();
    }
    pub fn dispatch(&self) {
        let mut ring = self.ring.lock().unwrap();
        ring.submit_and_wait(1).unwrap();
        let mut cqe_reaped = Vec::new();
        for cqe in ring.completion() {
            cqe_reaped.push(cqe.user_data() as RawFd);
        }
        let mut wakers = self.wakers.lock().unwrap();
        for fd in cqe_reaped {
            if let Some(waker) = wakers.remove(&fd) {
                waker.wake();
            }
        }
    }
}
pub struct AsyncTcpSocket {
    fd: RawFd,
    reactor: Arc<AsyncReactor>,
}
impl AsyncTcpSocket {
    pub fn new(fd: RawFd, reactor: Arc<AsyncReactor>) -> Self {
        Self { fd, reactor }
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let flags = unsafe { libc::fcntl(self.fd, libc::F_GETFL) };
        if flags < 0 {
            return Poll::Ready(Err(std::io::Error::last_os_error()));
        }
        let mut buf = [0u8; 1];
        let res = unsafe {
            libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, 1, libc::MSG_PEEK | libc::MSG_DONTWAIT)
        };
        if res >= 0 {
            Poll::Ready(Ok(()))
        } else {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) || err.raw_os_error() == Some(libc::EAGAIN) {
                self.reactor.register_interest(self.fd, cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(Err(err))
            }
        }
    }
}

