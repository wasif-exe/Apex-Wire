use crate::codec::{FixDecoder, FixEncoder, FixSeqNum};
use aw_exchange::fix::FixMessage;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Disconnected,
    LogonSent,
    Active,
    LogoutSent,
    ResendRequested,
}

pub struct FixSessionConfig {
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub heartbeat_secs: u32,
    pub reconnect_delay_ms: u64,
    pub logon_timeout_ms: u64,
}

impl Default for FixSessionConfig {
    fn default() -> Self {
        Self {
            sender_comp_id: "APEX".to_string(),
            target_comp_id: "EXCHANGE".to_string(),
            heartbeat_secs: 30,
            reconnect_delay_ms: 1000,
            logon_timeout_ms: 5000,
        }
    }
}

pub struct FixSession {
    config: FixSessionConfig,
    state: SessionState,
    encoder: FixEncoder,
    decoder: FixDecoder,
    stream: Option<TcpStream>,
    last_send: Instant,
    last_recv: Instant,
    recv_buf: Vec<u8>,
    recv_len: usize,
    send_buf: Vec<u8>,
}

impl FixSession {
    pub fn new(config: FixSessionConfig) -> Self {
        let encoder = FixEncoder::new(&config.sender_comp_id, &config.target_comp_id);
        let decoder = FixDecoder::new();
        Self {
            config,
            state: SessionState::Disconnected,
            encoder,
            decoder,
            stream: None,
            last_send: Instant::now(),
            last_recv: Instant::now(),
            recv_buf: vec![0u8; 65536],
            recv_len: 0,
            send_buf: vec![0u8; 65536],
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn out_seq(&self) -> FixSeqNum {
        self.encoder.out_seq()
    }

    pub fn in_seq(&self) -> FixSeqNum {
        self.decoder.in_seq()
    }

    pub fn connect(&mut self, addr: &str) -> io::Result<()> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        self.stream = Some(stream);
        self.state = SessionState::Disconnected;
        self.recv_len = 0;
        Ok(())
    }

    fn flush_send(&mut self, n: usize) -> io::Result<()> {
        if let Some(ref mut stream) = self.stream {
            stream.write_all(&self.send_buf[..n])?;
        }
        self.last_send = Instant::now();
        Ok(())
    }

    pub fn send_logon(&mut self) -> io::Result<()> {
        let n = self.encoder.encode_logon(&mut self.send_buf, self.config.heartbeat_secs);
        self.flush_send(n)?;
        self.state = SessionState::LogonSent;
        Ok(())
    }

    pub fn send_heartbeat(&mut self) -> io::Result<()> {
        let n = self.encoder.encode_heartbeat(&mut self.send_buf);
        self.flush_send(n)
    }

    pub fn send_logout(&mut self) -> io::Result<()> {
        let n = self.encoder.encode_logout(&mut self.send_buf);
        self.flush_send(n)?;
        self.state = SessionState::LogoutSent;
        Ok(())
    }

    pub fn send_new_order(
        &mut self,
        cl_ord_id: &[u8],
        symbol: &[u8],
        side: u8,
        qty: u64,
        price: &[u8],
        ord_type: u8,
        tif: u8,
    ) -> io::Result<()> {
        let n = self.encoder.encode_new_order_single(
            &mut self.send_buf,
            cl_ord_id,
            symbol,
            side,
            qty,
            price,
            ord_type,
            tif,
        );
        self.flush_send(n)
    }

    pub fn poll(&mut self) -> io::Result<Vec<FixMessageOwned>> {
        let mut results = Vec::new();

        if let Some(ref mut stream) = self.stream {
            match stream.read(&mut self.recv_buf[self.recv_len..]) {
                Ok(0) => {
                    self.state = SessionState::Disconnected;
                    return Ok(results);
                }
                Ok(n) => {
                    self.recv_len += n;
                    self.last_recv = Instant::now();
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e),
            }
        }

        while self.recv_len > 0 {
            let mut msg = FixMessage::default();
            match self.decoder.decode(&self.recv_buf[..self.recv_len], &mut msg) {
                Some(consumed) => {
                    let owned = fix_to_owned(&msg);
                    results.push(owned);

                    if msg.msg_type == b"A" && self.state == SessionState::LogonSent {
                        self.state = SessionState::Active;
                    } else if msg.msg_type == b"5" {
                        self.state = SessionState::Disconnected;
                    }

                    self.recv_buf.copy_within(consumed..self.recv_len, 0);
                    self.recv_len -= consumed;
                }
                None => break,
            }
        }

        if self.state == SessionState::Active {
            let hb = Duration::from_secs(self.config.heartbeat_secs as u64);
            if self.last_send.elapsed() > hb {
                let _ = self.send_heartbeat();
            }
        }

        Ok(results)
    }

    pub fn disconnect(&mut self) {
        if self.state == SessionState::Active {
            let _ = self.send_logout();
        }
        self.stream = None;
        self.state = SessionState::Disconnected;
    }
}

#[derive(Debug, Clone)]
pub struct FixMessageOwned {
    pub msg_type: Vec<u8>,
    pub tags: Vec<(u32, Vec<u8>)>,
}

impl FixMessageOwned {
    pub fn get_tag(&self, tag: u32) -> Option<&[u8]> {
        self.tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| v.as_slice())
    }
}

fn fix_to_owned(msg: &FixMessage<'_>) -> FixMessageOwned {
    let mut tags = Vec::with_capacity(msg.count);
    for i in 0..msg.count {
        tags.push((msg.fields[i].tag, msg.fields[i].val.to_vec()));
    }
    FixMessageOwned {
        msg_type: msg.msg_type.to_vec(),
        tags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_initial_state() {
        let cfg = FixSessionConfig::default();
        let session = FixSession::new(cfg);
        assert_eq!(session.state(), SessionState::Disconnected);
        assert_eq!(session.out_seq(), 1);
        assert_eq!(session.in_seq(), 1);
    }
}
