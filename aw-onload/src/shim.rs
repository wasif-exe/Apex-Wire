use std::cell::UnsafeCell;
use std::os::raw::{c_int, c_void};
use crate::fd_table::{FdTable, FdType};
thread_local! {
    static THREAD_FD_TABLE: UnsafeCell<FdTable> = UnsafeCell::new(FdTable::new());
}
pub fn get_fd_table() -> &'static mut FdTable {
    // SAFETY: Thread-local UnsafeCell retrieval for non-aliasing thread-safe access.
    THREAD_FD_TABLE.with(|cell| unsafe { &mut *cell.get() })
}
#[no_mangle]
pub unsafe extern "C" fn aw_socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int {
    let raw_fd = libc::socket(domain, socket_type, protocol);
    if raw_fd >= 0 && (domain == libc::AF_INET || domain == libc::AF_INET6) {
        let table = get_fd_table();
        table.register(raw_fd, FdType::WireIntercepted);
    }
    raw_fd
}
#[no_mangle]
pub unsafe extern "C" fn aw_bind(sockfd: c_int, addr: *const libc::sockaddr, addrlen: libc::socklen_t) -> c_int {
    libc::bind(sockfd, addr, addrlen)
}
#[no_mangle]
pub unsafe extern "C" fn aw_connect(sockfd: c_int, addr: *const libc::sockaddr, addrlen: libc::socklen_t) -> c_int {
    libc::connect(sockfd, addr, addrlen)
}
#[no_mangle]
pub unsafe extern "C" fn aw_send(sockfd: c_int, buf: *const c_void, len: usize, flags: c_int) -> isize {
    let table = get_fd_table();
    if table.is_intercepted(sockfd) {
        let slice = std::slice::from_raw_parts(buf as *const u8, len);
        let _fake_crc = slice.len();
        len as isize
    } else {
        libc::send(sockfd, buf, len, flags)
    }
}
#[no_mangle]
pub unsafe extern "C" fn aw_recv(sockfd: c_int, buf: *mut c_void, len: usize, flags: c_int) -> isize {
    let table = get_fd_table();
    if table.is_intercepted(sockfd) {
        0
    } else {
        libc::recv(sockfd, buf, len, flags)
    }
}
#[no_mangle]
pub unsafe extern "C" fn aw_close(fd: c_int) -> c_int {
    let table = get_fd_table();
    if table.is_intercepted(fd) {
        table.unregister(fd);
        0
    } else {
        libc::close(fd)
    }
}
#[no_mangle]
pub unsafe extern "C" fn aw_epoll_create1(flags: c_int) -> c_int {
    libc::epoll_create1(flags)
}
#[no_mangle]
pub unsafe extern "C" fn aw_epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: *mut libc::epoll_event) -> c_int {
    libc::epoll_ctl(epfd, op, fd, event)
}
#[no_mangle]
pub unsafe extern "C" fn aw_epoll_wait(epfd: c_int, events: *mut libc::epoll_event, maxevents: c_int, timeout: c_int) -> c_int {
    libc::epoll_wait(epfd, events, maxevents, timeout)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_shim_interception() {
        unsafe {
            let fake_buf = b"GET / HTTP/1.1\r\n\r\n";
            let intercepted_fd = 100;
            let table = get_fd_table();
            table.register(intercepted_fd, FdType::WireIntercepted);
            let sent = aw_send(intercepted_fd, fake_buf.as_ptr() as *const c_void, fake_buf.len(), 0);
            assert_eq!(sent, fake_buf.len() as isize);
            let closed = aw_close(intercepted_fd);
            assert_eq!(closed, 0);
            assert!(!table.is_intercepted(intercepted_fd));
        }
    }
}
