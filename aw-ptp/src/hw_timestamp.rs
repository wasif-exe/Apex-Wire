use std::os::unix::io::RawFd;
pub const SOF_TIMESTAMPING_TX_HARDWARE: u32 = 1 << 0;
pub const SOF_TIMESTAMPING_TX_SOFTWARE: u32 = 1 << 1;
pub const SOF_TIMESTAMPING_RX_HARDWARE: u32 = 1 << 2;
pub const SOF_TIMESTAMPING_RX_SOFTWARE: u32 = 1 << 3;
pub const SOF_TIMESTAMPING_SOFTWARE: u32 = 1 << 4;
pub const SOF_TIMESTAMPING_RAW_HARDWARE: u32 = 1 << 6;
pub const SOF_TIMESTAMPING_OPT_ID: u32 = 1 << 7;
pub const SOF_TIMESTAMPING_TX_SCHED: u32 = 1 << 8;
pub const SOF_TIMESTAMPING_TX_ACK: u32 = 1 << 9;
pub const SOF_TIMESTAMPING_OPT_CMSG: u32 = 1 << 10;
pub const SOF_TIMESTAMPING_OPT_TSONLY: u32 = 1 << 11;
pub const SO_TIMESTAMPING: libc::c_int = 37;
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HwTimestamp {
    pub sec: u64,
    pub nsec: u32,
    pub is_hardware: bool,
}
impl HwTimestamp {
    #[inline(always)]
    pub fn as_nanos(&self) -> u64 {
        self.sec.wrapping_mul(1_000_000_000).wrapping_add(self.nsec as u64)
    }
}
pub fn enable_hardware_timestamping(fd: RawFd) -> anyhow::Result<bool> {
    let flags = SOF_TIMESTAMPING_RX_HARDWARE
        | SOF_TIMESTAMPING_TX_HARDWARE
        | SOF_TIMESTAMPING_RAW_HARDWARE
        | SOF_TIMESTAMPING_SOFTWARE
        | SOF_TIMESTAMPING_RX_SOFTWARE
        | SOF_TIMESTAMPING_TX_SOFTWARE
        | SOF_TIMESTAMPING_OPT_TSONLY;
    // SAFETY: Invoking setsockopt with SO_TIMESTAMPING flags to configure NIC clock latching.
    unsafe {
        let ret = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_TIMESTAMPING,
            &flags as *const _ as *const libc::c_void,
            std::mem::size_of::<u32>() as libc::socklen_t,
        );
        if ret == 0 {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
pub fn read_ptp_timestamp_cmsg(msghdr: &libc::msghdr) -> Option<HwTimestamp> {
    // SAFETY: Traversal of control message (cmsg) linked list to extract SCM_TIMESTAMPING.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(msghdr);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == SO_TIMESTAMPING {
                let data_ptr = libc::CMSG_DATA(cmsg) as *const libc::timespec;
                if !data_ptr.is_null() {
                    let hw_ts = *data_ptr.add(2);
                    if hw_ts.tv_sec != 0 || hw_ts.tv_nsec != 0 {
                        return Some(HwTimestamp {
                            sec: hw_ts.tv_sec as u64,
                            nsec: hw_ts.tv_nsec as u32,
                            is_hardware: true,
                        });
                    }
                    let sw_ts = *data_ptr.add(0);
                    if sw_ts.tv_sec != 0 || sw_ts.tv_nsec != 0 {
                        return Some(HwTimestamp {
                            sec: sw_ts.tv_sec as u64,
                            nsec: sw_ts.tv_nsec as u32,
                            is_hardware: false,
                        });
                    }
                }
            }
            cmsg = libc::CMSG_NXTHDR(msghdr as *const _ as *mut _, cmsg);
        }
    }
    None
}
pub fn probe_ptp_hardware_support(ifname: &str) -> bool {
    let dev_path = format!("/sys/class/net/{}/device/ptp", ifname);
    std::path::Path::new(&dev_path).exists()
}

