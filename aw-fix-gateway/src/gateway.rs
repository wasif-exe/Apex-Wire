use crate::session::{FixSession, FixSessionConfig, SessionState};
use aw_disruptor::Disruptor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct OrderCommand {
    pub action: u8,
    pub side: u8,
    pub _pad: [u8; 2],
    pub price: u32,
    pub shares: u32,
    pub order_id: u64,
    pub symbol: [u8; 8],
    pub cl_ord_id: [u8; 16],
}

impl Default for OrderCommand {
    #[inline(always)]
    fn default() -> Self {
        Self {
            action: 0,
            side: 0,
            _pad: [0; 2],
            price: 0,
            shares: 0,
            order_id: 0,
            symbol: [0; 8],
            cl_ord_id: [0; 16],
        }
    }
}

pub const CMD_NEW: u8 = 1;
pub const CMD_CANCEL: u8 = 2;
pub const CMD_REPLACE: u8 = 3;

pub struct FixGateway {
    addr: String,
    config: FixSessionConfig,
}

impl FixGateway {
    pub fn new(addr: &str, config: FixSessionConfig) -> Self {
        Self {
            addr: addr.to_string(),
            config,
        }
    }

    pub fn spawn_order_thread(
        self,
        cmd_ring: &'static Disruptor<OrderCommand>,
        running: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<GatewayStats> {
        std::thread::spawn(move || {
            let mut session = FixSession::new(self.config);
            let mut cons = cmd_ring.consumer();
            let mut stats = GatewayStats::default();

            while running.load(Ordering::Acquire) {
                if session.state() == SessionState::Disconnected {
                    if session.connect(&self.addr).is_ok() {
                        let _ = session.send_logon();
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        stats.connect_failures += 1;
                        continue;
                    }
                }

                let _ = cons.drain(64, |cmd| {
                    if session.state() != SessionState::Active {
                        return;
                    }
                    match cmd.action {
                        CMD_NEW => {
                            let price_str = format_price(cmd.price);
                            let _ = session.send_new_order(
                                &cmd.cl_ord_id,
                                &cmd.symbol,
                                cmd.side,
                                cmd.shares as u64,
                                &price_str,
                                b'2',
                                b'0',
                            );
                            stats.orders_sent += 1;
                        }
                        CMD_CANCEL => {
                            stats.cancels_sent += 1;
                        }
                        _ => {}
                    }
                });

                match session.poll() {
                    Ok(msgs) => {
                        for msg in &msgs {
                            if msg.msg_type == b"8" {
                                stats.exec_reports += 1;
                            }
                        }
                        stats.messages_received += msgs.len() as u64;
                    }
                    Err(_) => {
                        stats.errors += 1;
                    }
                }
            }

            session.disconnect();
            stats
        })
    }
}

#[derive(Default)]
pub struct GatewayStats {
    pub orders_sent: u64,
    pub cancels_sent: u64,
    pub exec_reports: u64,
    pub messages_received: u64,
    pub connect_failures: u64,
    pub errors: u64,
}

fn format_price(price: u32) -> [u8; 8] {
    let mut buf = [b' '; 8];
    let dollars = price / 10000;
    let cents = price % 10000;
    let d_str = format_u32_simple(dollars);
    let c_str = format_u32_padded(cents, 4);
    let mut pos = 0;
    for &b in &d_str {
        if b != 0 && pos < 8 { buf[pos] = b; pos += 1; }
    }
    if pos < 8 { buf[pos] = b'.'; pos += 1; }
    for &b in &c_str {
        if pos < 8 { buf[pos] = b; pos += 1; }
    }
    buf
}

fn format_u32_simple(v: u32) -> [u8; 6] {
    let mut buf = [0u8; 6];
    if v == 0 { buf[0] = b'0'; return buf; }
    let mut pos = 6;
    let mut n = v;
    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let mut out = [0u8; 6];
    let len = 6 - pos;
    out[..len].copy_from_slice(&buf[pos..]);
    out
}

fn format_u32_padded(v: u32, width: usize) -> [u8; 4] {
    let mut buf = [b'0'; 4];
    let mut n = v;
    for i in (0..width).rev() {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_price_basic() {
        let p = format_price(1855000);
        let s = std::str::from_utf8(&p).unwrap();
        assert!(s.starts_with("185.5000"));
    }

    #[test]
    fn order_command_default() {
        let cmd = OrderCommand::default();
        assert_eq!(cmd.action, 0);
        assert_eq!(cmd.shares, 0);
    }
}
