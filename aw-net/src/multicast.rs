use crate::recvmmsg::BatchReceiver;
use aw_disruptor::Disruptor;
use aw_exchange::itch::parse_itch_message;
use aw_exchange::order::MarketEvent;
use aw_pcap::{extract_udp_payload, parse_moldudp64_header, MoldUdp64Iter, MOLDUDP64_HEARTBEAT};
use std::io;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct MulticastConfig {
    pub multicast_addr: Ipv4Addr,
    pub port: u16,
    pub interface_addr: Ipv4Addr,
    pub batch_size: usize,
    pub recv_buf_mb: usize,
}

impl Default for MulticastConfig {
    fn default() -> Self {
        Self {
            multicast_addr: Ipv4Addr::new(224, 0, 0, 1),
            port: 15000,
            interface_addr: Ipv4Addr::new(0, 0, 0, 0),
            batch_size: 32,
            recv_buf_mb: 16,
        }
    }
}

pub struct MulticastReceiver {
    fd: i32,
    config: MulticastConfig,
}

impl MulticastReceiver {
    pub fn bind(config: MulticastConfig) -> io::Result<Self> {
        // SAFETY: socket() with valid AF_INET, SOCK_DGRAM, 0.
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let reuse = 1i32;
        // SAFETY: setsockopt with valid fd and SO_REUSEADDR.
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                &reuse as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }

        let buf_size = (config.recv_buf_mb * 1024 * 1024) as i32;
        // SAFETY: setsockopt with valid fd and SO_RCVBUF.
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &buf_size as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }

        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as u16,
            sin_port: config.port.to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from(Ipv4Addr::UNSPECIFIED).to_be(),
            },
            sin_zero: [0; 8],
        };
        // SAFETY: bind with properly initialized sockaddr_in.
        let ret = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as u32,
            )
        };
        if ret < 0 {
            let err = io::Error::last_os_error();
            // SAFETY: closing fd on error path.
            unsafe { libc::close(fd); }
            return Err(err);
        }

        let mreq = libc::ip_mreq {
            imr_multiaddr: libc::in_addr {
                s_addr: u32::from(config.multicast_addr).to_be(),
            },
            imr_interface: libc::in_addr {
                s_addr: u32::from(config.interface_addr).to_be(),
            },
        };
        // SAFETY: setsockopt with valid fd and IP_ADD_MEMBERSHIP.
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_ADD_MEMBERSHIP,
                &mreq as *const libc::ip_mreq as *const libc::c_void,
                std::mem::size_of::<libc::ip_mreq>() as u32,
            )
        };
        if ret < 0 {
            let err = io::Error::last_os_error();
            // SAFETY: closing fd on error path.
            unsafe { libc::close(fd); }
            return Err(err);
        }

        Ok(Self { fd, config })
    }

    pub fn spawn_ingest_thread(
        self,
        ring: &'static Disruptor<MarketEvent>,
        running: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<IngestStats> {
        std::thread::spawn(move || {
            let mut batch = BatchReceiver::new(self.fd, self.config.batch_size);
            let pub_handle = ring.publisher();
            let mut stats = IngestStats::default();
            let mut scratch;

            while running.load(Ordering::Acquire) {
                let n = match batch.recv_batch() {
                    Ok(n) => n,
                    Err(_) => {
                        stats.errors += 1;
                        continue;
                    }
                };

                if n == 0 {
                    std::hint::spin_loop();
                    continue;
                }

                stats.packets += n as u64;

                for pkt in &batch.packets()[..n] {
                    if pkt.len < 42 {
                        continue;
                    }

                    let udp_payload = match extract_udp_payload(&pkt.data[..pkt.len]) {
                        Some(p) => p,
                        None => continue,
                    };

                    let hdr = match parse_moldudp64_header(udp_payload) {
                        Some(h) => h,
                        None => continue,
                    };

                    stats.mold_messages += hdr.message_count as u64;

                    if hdr.message_count == MOLDUDP64_HEARTBEAT {
                        stats.heartbeats += 1;
                        continue;
                    }

                    let iter = MoldUdp64Iter::new(udp_payload, hdr.message_count);
                    for msg in iter {
                        scratch = MarketEvent::default();
                        if parse_itch_message(msg, &mut scratch) {
                            let seq = pub_handle.claim_spin();
                            *pub_handle.get_mut(seq) = scratch;
                            pub_handle.publish(seq);
                            stats.itch_events += 1;
                        } else {
                            stats.parse_failures += 1;
                        }
                    }
                }
            }

            stats
        })
    }

    pub fn fd(&self) -> i32 {
        self.fd
    }
}

impl Drop for MulticastReceiver {
    fn drop(&mut self) {
        if self.fd >= 0 {
            let mreq = libc::ip_mreq {
                imr_multiaddr: libc::in_addr {
                    s_addr: u32::from(self.config.multicast_addr).to_be(),
                },
                imr_interface: libc::in_addr {
                    s_addr: u32::from(self.config.interface_addr).to_be(),
                },
            };
            // SAFETY: leaving multicast group on drop.
            unsafe {
                libc::setsockopt(
                    self.fd,
                    libc::IPPROTO_IP,
                    libc::IP_DROP_MEMBERSHIP,
                    &mreq as *const libc::ip_mreq as *const libc::c_void,
                    std::mem::size_of::<libc::ip_mreq>() as u32,
                );
                libc::close(self.fd);
            }
            self.fd = -1;
        }
    }
}

#[derive(Default)]
pub struct IngestStats {
    pub packets: u64,
    pub mold_messages: u64,
    pub itch_events: u64,
    pub heartbeats: u64,
    pub parse_failures: u64,
    pub errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let cfg = MulticastConfig::default();
        assert_eq!(cfg.port, 15000);
        assert_eq!(cfg.batch_size, 32);
        assert_eq!(cfg.recv_buf_mb, 16);
    }
}
