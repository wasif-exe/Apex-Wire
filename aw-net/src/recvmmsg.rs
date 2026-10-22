use std::io;

pub const MAX_BATCH: usize = 64;
pub const MAX_PKT_SIZE: usize = 9000;

#[repr(C, align(64))]
pub struct PacketBuf {
    pub data: [u8; MAX_PKT_SIZE],
    pub len: usize,
    pub timestamp_ns: u64,
}

impl Default for PacketBuf {
    #[inline(always)]
    fn default() -> Self {
        Self {
            data: [0; MAX_PKT_SIZE],
            len: 0,
            timestamp_ns: 0,
        }
    }
}

pub struct BatchReceiver {
    fd: i32,
    iovecs: Vec<libc::iovec>,
    msgs: Vec<libc::mmsghdr>,
    bufs: Vec<PacketBuf>,
    batch_size: usize,
}

impl BatchReceiver {
    pub fn new(fd: i32, batch_size: usize) -> Self {
        let bs = batch_size.min(MAX_BATCH);
        let mut bufs = Vec::with_capacity(bs);
        let mut iovecs = Vec::with_capacity(bs);
        let mut msgs = Vec::with_capacity(bs);

        for _ in 0..bs {
            bufs.push(PacketBuf::default());
        }

        for i in 0..bs {
            iovecs.push(libc::iovec {
                iov_base: bufs[i].data.as_mut_ptr() as *mut libc::c_void,
                iov_len: MAX_PKT_SIZE,
            });
            let mut hdr: libc::mmsghdr = unsafe { std::mem::zeroed() };
            hdr.msg_hdr.msg_iov = &mut iovecs[i] as *mut libc::iovec;
            hdr.msg_hdr.msg_iovlen = 1;
            msgs.push(hdr);
        }

        Self {
            fd,
            iovecs,
            msgs,
            bufs,
            batch_size: bs,
        }
    }

    #[inline]
    pub fn recv_batch(&mut self) -> io::Result<usize> {
        for i in 0..self.batch_size {
            self.iovecs[i].iov_base = self.bufs[i].data.as_mut_ptr() as *mut libc::c_void;
            self.iovecs[i].iov_len = MAX_PKT_SIZE;
            self.msgs[i].msg_hdr.msg_iov = &mut self.iovecs[i] as *mut libc::iovec;
            self.msgs[i].msg_hdr.msg_iovlen = 1;
            self.msgs[i].msg_len = 0;
        }

        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 1_000_000,
        };

        // SAFETY: recvmmsg called with valid fd, properly initialized mmsghdr array.
        let n = unsafe {
            libc::recvmmsg(
                self.fd,
                self.msgs.as_mut_ptr(),
                self.batch_size as u32,
                libc::MSG_DONTWAIT,
                &timeout as *const libc::timespec as *mut libc::timespec,
            )
        };

        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(0);
            }
            return Err(err);
        }

        let count = n as usize;
        for i in 0..count {
            self.bufs[i].len = self.msgs[i].msg_len as usize;
            self.bufs[i].timestamp_ns = 0;
        }

        Ok(count)
    }

    #[inline]
    pub fn packets(&self) -> &[PacketBuf] {
        &self.bufs
    }

    pub fn fd(&self) -> i32 {
        self.fd
    }
}

impl Drop for BatchReceiver {
    fn drop(&mut self) {
        if self.fd >= 0 {
            // SAFETY: closing a valid file descriptor.
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}
