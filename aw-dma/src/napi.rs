use std::os::unix::io::RawFd;
pub const SO_BUSY_POLL: libc::c_int = 46;
pub const SO_PREFER_BUSY_POLL: libc::c_int = 69;
pub const SO_BUSY_POLL_BUDGET: libc::c_int = 70;
#[derive(Debug, Clone, Copy)]
pub struct NapiBusyPollConfig {
    pub busy_poll_usecs: u32,
    pub prefer_busy_poll: bool,
    pub poll_budget: u32,
}
impl Default for NapiBusyPollConfig {
    fn default() -> Self {
        Self {
            busy_poll_usecs: 50,
            prefer_busy_poll: true,
            poll_budget: 64,
        }
    }
}
pub fn configure_napi_busy_poll(fd: RawFd, config: &NapiBusyPollConfig) -> anyhow::Result<bool> {
    // SAFETY: Invoking setsockopt with SO_BUSY_POLL, SO_PREFER_BUSY_POLL, SO_BUSY_POLL_BUDGET.
    unsafe {
        let usecs = config.busy_poll_usecs as libc::c_int;
        let ret = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_BUSY_POLL,
            &usecs as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if ret != 0 {
            return Ok(false);
        }
        if config.prefer_busy_poll {
            let pref: libc::c_int = 1;
            let _ = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                SO_PREFER_BUSY_POLL,
                &pref as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
        if config.poll_budget > 0 {
            let budget = config.poll_budget as libc::c_int;
            let _ = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                SO_BUSY_POLL_BUDGET,
                &budget as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
    Ok(true)
}

