use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::io::Result;
#[repr(C)]
struct IfReq {
    ifr_name: [u8; 16],
    ifr_flags: u16,
    _pad: [u8; 22],
}
const IFF_TAP: u16 = 0x0002;
const IFF_NO_PI: u16 = 0x1000;
const TUNSETIFF: libc::c_ulong = 0x400454ca;
pub struct TapDevice {
    file: std::fs::File,
}
impl TapDevice {
    pub fn new(name: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")?;
        let mut req = IfReq {
            ifr_name: [0; 16],
            ifr_flags: IFF_TAP | IFF_NO_PI,
            _pad: [0; 22],
        };
        let bytes = name.as_bytes();
        let len = bytes.len().min(15);
        req.ifr_name[..len].copy_from_slice(&bytes[..len]);
        let ret = unsafe { libc::ioctl(file.as_raw_fd(), TUNSETIFF, &mut req) };
        if ret < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe {
            let flags = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        Ok(Self { file })
    }
    pub fn poll_read(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        let n = unsafe {
            libc::read(
                self.file.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n > 0 {
            Ok(Some(n as usize))
        } else if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) || err.raw_os_error() == Some(libc::EAGAIN) {
                Ok(None)
            } else {
                Err(err)
            }
        } else {
            Ok(None)
        }
    }
    pub fn write_async(&mut self, buf: &[u8]) -> Result<Option<usize>> {
    let n = unsafe {
        libc::write(
            self.file.as_raw_fd(),
            buf.as_ptr() as *const libc::c_void,
            buf.len(),
        )
    };
    if n >= 0 {
        Ok(Some(n as usize))
    } else {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) || err.raw_os_error() == Some(libc::EAGAIN) {
            Ok(None)
        } else {
            Err(err)
        }
    }
}
}
