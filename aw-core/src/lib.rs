pub mod profile;
pub mod budget;
pub mod conntable;
pub mod types;
pub mod shard;
pub mod cacheline;
pub mod resp;
pub mod kv;
pub use profile::probes;
pub use budget::{LatencyBudget, BudgetReport};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddress(pub [u8; 6]);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4Address(pub [u8; 4]);
impl From<Ipv4Addr> for Ipv4Address {
    fn from(addr: Ipv4Addr) -> Self {
        Self(addr.octets())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(pub u32);
impl Seq {
    pub fn wrapping_add(self, val: u32) -> Self {
        Seq(self.0.wrapping_add(val))
    }
    pub fn wrapping_sub(self, val: Seq) -> u32 {
        self.0.wrapping_sub(val.0)
    }
    pub fn lt(self, other: Seq) -> bool {
        (self.0.wrapping_sub(other.0) as i32) < 0
    }
    pub fn lte(self, other: Seq) -> bool {
        self == other || self.lt(other)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TcpFlags: u8 {
        const FIN = 0x01;
        const SYN = 0x02;
        const RST = 0x04;
        const PSH = 0x08;
        const ACK = 0x10;
        const URG = 0x20;
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TcpOption {
    Mss(u16),
    WindowScale(u8),
    SackPermitted,
    Sack(Vec<(Seq, Seq)>),
    Timestamp { tsval: u32, tsecr: u32 },
    Nop,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UdpTuple {
    pub local_ip: Ipv4Address,
    pub local_port: u16,
    pub remote_ip: Ipv4Address,
    pub remote_port: u16,
}
#[derive(Debug)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}
impl UdpHeader {
    pub fn parse(buf: &[u8]) -> Option<(Self, &[u8])> {
        if buf.len() < 8 { return None; }
        let src_port = u16::from_be_bytes([buf[0], buf[1]]);
        let dst_port = u16::from_be_bytes([buf[2], buf[3]]);
        let length = u16::from_be_bytes([buf[4], buf[5]]);
        let checksum = u16::from_be_bytes([buf[6], buf[7]]);
        if buf.len() < length as usize || length < 8 { return None; }
        Some((UdpHeader { src_port, dst_port, length, checksum }, &buf[8..length as usize]))
    }
    pub fn serialize(&self) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0..2].copy_from_slice(&self.src_port.to_be_bytes());
        buf[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
        buf[4..6].copy_from_slice(&self.length.to_be_bytes());
        buf[6..8].copy_from_slice(&self.checksum.to_be_bytes());
        buf
    }
}
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]]) as u32
        } else {
            (chunk[0] as u32) << 8
        };
        sum += word;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}
pub fn tcp_checksum(src: Ipv4Address, dst: Ipv4Address, tcp_segment: &[u8]) -> u16 {
    let mut pseudo_hdr = Vec::with_capacity(12 + tcp_segment.len());
    pseudo_hdr.extend_from_slice(&src.0);
    pseudo_hdr.extend_from_slice(&dst.0);
    pseudo_hdr.push(0);
    pseudo_hdr.push(6);
    pseudo_hdr.extend_from_slice(&(tcp_segment.len() as u16).to_be_bytes());
    pseudo_hdr.extend_from_slice(tcp_segment);
    checksum(&pseudo_hdr)
}
pub fn udp_checksum(src: Ipv4Address, dst: Ipv4Address, udp_segment: &[u8]) -> u16 {
    let mut pseudo_hdr = Vec::with_capacity(12 + udp_segment.len());
    pseudo_hdr.extend_from_slice(&src.0);
    pseudo_hdr.extend_from_slice(&dst.0);
    pseudo_hdr.push(0);
    pseudo_hdr.push(17);
    pseudo_hdr.extend_from_slice(&(udp_segment.len() as u16).to_be_bytes());
    pseudo_hdr.extend_from_slice(udp_segment);
    let res = checksum(&pseudo_hdr);
    if res == 0 { 0xFFFF } else { res }
}
pub fn build_dns_query(hostname: &str, tx_id: u16) -> Vec<u8> {
    let mut pkt = Vec::new();
    pkt.extend_from_slice(&tx_id.to_be_bytes());
    pkt.extend_from_slice(&0x0100u16.to_be_bytes());
    pkt.extend_from_slice(&1u16.to_be_bytes());
    pkt.extend_from_slice(&0u16.to_be_bytes());
    pkt.extend_from_slice(&0u16.to_be_bytes());
    pkt.extend_from_slice(&0u16.to_be_bytes());
    for label in hostname.split('.') {
        pkt.push(label.len() as u8);
        pkt.extend_from_slice(label.as_bytes());
    }
    pkt.push(0);
    pkt.extend_from_slice(&1u16.to_be_bytes());
    pkt.extend_from_slice(&1u16.to_be_bytes());
    pkt
}
pub fn parse_dns_response(buf: &[u8]) -> Option<Ipv4Address> {
    if buf.len() < 12 { return None; }
    let ancount = u16::from_be_bytes([buf[6], buf[7]]);
    if ancount == 0 { return None; }
    let mut idx = 12;
    while idx < buf.len() && buf[idx] != 0 {
        idx += (buf[idx] as usize) + 1;
    }
    idx += 5;
    for _ in 0..ancount {
        if idx >= buf.len() { break; }
        if (buf[idx] & 0xC0) == 0xC0 {
            idx += 2;
        } else {
            while idx < buf.len() && buf[idx] != 0 {
                idx += (buf[idx] as usize) + 1;
            }
            idx += 1;
        }
        if idx + 10 > buf.len() { break; }
        let rtype = u16::from_be_bytes([buf[idx], buf[idx + 1]]);
        let rdlength = u16::from_be_bytes([buf[idx + 8], buf[idx + 9]]) as usize;
        idx += 10;
        if rtype == 1 && rdlength == 4 && idx + 4 <= buf.len() {
            return Some(Ipv4Address([buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]]));
        }
        idx += rdlength;
    }
    None
}
#[derive(Debug)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: Seq,
    pub ack: Seq,
    pub data_offset: u8,
    pub flags: TcpFlags,
    pub window: u16,
    pub checksum: u16,
    pub options: Vec<TcpOption>,
}
impl TcpHeader {
    pub fn parse(buf: &[u8]) -> Option<(Self, &[u8])> {
        if buf.len() < 20 { return None; }
        let src_port = u16::from_be_bytes([buf[0], buf[1]]);
        let dst_port = u16::from_be_bytes([buf[2], buf[3]]);
        let seq = Seq(u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]));
        let ack = Seq(u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]));
        let data_offset_words = buf[12] >> 4;
        let data_offset_bytes = data_offset_words as usize * 4;
        let flags = TcpFlags::from_bits_retain(buf[13]);
        let window = u16::from_be_bytes([buf[14], buf[15]]);
        let checksum = u16::from_be_bytes([buf[16], buf[17]]);
        if data_offset_bytes < 20 || buf.len() < data_offset_bytes { return None; }
        let mut options = Vec::new();
        if data_offset_bytes > 20 {
            let opt_len = data_offset_bytes - 20;
            let opt_buf = &buf[20..20 + opt_len];
            let mut i = 0;
            while i < opt_buf.len() {
                let kind = opt_buf[i];
                if kind == 0 { break; }
                if kind == 1 {
                    options.push(TcpOption::Nop);
                    i += 1;
                    continue;
                }
                if i + 1 >= opt_buf.len() { break; }
                let len = opt_buf[i + 1] as usize;
                if len < 2 || i + len > opt_buf.len() { break; }
                let data = &opt_buf[i + 2..i + len];
                match kind {
                    2 => {
                        if data.len() == 2 {
                            options.push(TcpOption::Mss(u16::from_be_bytes([data[0], data[1]])));
                        }
                    }
                    3 => {
                        if data.len() == 1 {
                            options.push(TcpOption::WindowScale(data[0]));
                        }
                    }
                    4 => {
                        if data.len() == 0 {
                            options.push(TcpOption::SackPermitted);
                        }
                    }
                    5 => {
                        if data.len() % 8 == 0 {
                            let mut blocks = Vec::new();
                            for chunk in data.chunks_exact(8) {
                                let start = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                                let end = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                                blocks.push((Seq(start), Seq(end)));
                            }
                            options.push(TcpOption::Sack(blocks));
                        }
                    }
                    8 => {
                        if data.len() == 8 {
                            let tsval = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                            let tsecr = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                            options.push(TcpOption::Timestamp { tsval, tsecr });
                        }
                    }
                    _ => {}
                }
                i += len;
            }
        }
        Some((TcpHeader {
            src_port, dst_port, seq, ack, data_offset: data_offset_words, flags, window, checksum, options,
        }, &buf[data_offset_bytes..]))
    }
    pub fn serialize(&self) -> Vec<u8> {
        let mut opt_buf = Vec::new();
        for opt in &self.options {
            match opt {
                TcpOption::Nop => { opt_buf.push(1); }
                TcpOption::Mss(mss) => {
                    opt_buf.push(2);
                    opt_buf.push(4);
                    opt_buf.extend_from_slice(&mss.to_be_bytes());
                }
                TcpOption::WindowScale(scale) => {
                    opt_buf.push(3);
                    opt_buf.push(3);
                    opt_buf.push(*scale);
                }
                TcpOption::SackPermitted => {
                    opt_buf.push(4);
                    opt_buf.push(2);
                }
                TcpOption::Sack(blocks) => {
                    let len = 2 + blocks.len() * 8;
                    opt_buf.push(5);
                    opt_buf.push(len as u8);
                    for (start, end) in blocks {
                        opt_buf.extend_from_slice(&start.0.to_be_bytes());
                        opt_buf.extend_from_slice(&end.0.to_be_bytes());
                    }
                }
                TcpOption::Timestamp { tsval, tsecr } => {
                    opt_buf.push(8);
                    opt_buf.push(10);
                    opt_buf.extend_from_slice(&tsval.to_be_bytes());
                    opt_buf.extend_from_slice(&tsecr.to_be_bytes());
                }
            }
        }
        while opt_buf.len() % 4 != 0 {
            opt_buf.push(0);
        }
        let data_offset = 5 + (opt_buf.len() / 4) as u8;
        let mut buf = vec![0u8; 20];
        buf[0..2].copy_from_slice(&self.src_port.to_be_bytes());
        buf[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
        buf[4..8].copy_from_slice(&self.seq.0.to_be_bytes());
        buf[8..12].copy_from_slice(&self.ack.0.to_be_bytes());
        buf[12] = data_offset << 4;
        buf[13] = self.flags.bits();
        buf[14..16].copy_from_slice(&self.window.to_be_bytes());
        buf.extend_from_slice(&opt_buf);
        buf
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketTuple {
    pub local_ip: Ipv4Address,
    pub local_port: u16,
    pub remote_ip: Ipv4Address,
    pub remote_port: u16,
}
pub struct SentSegment {
    pub seq: Seq,
    pub seq_len: u32,
    pub data: Vec<u8>,
    pub sent_at: Instant,
    pub retransmit_count: u32,
    pub sacked: bool,
    pub lost: bool,
    pub delivered_at_send: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BbrState {
    Startup,
    Drain,
    ProbeBw,
    ProbeRtt,
}
pub struct TcpConnection {
    pub tuple: SocketTuple,
    pub state: TcpState,
    pub local_mac: MacAddress,
    pub snd_una: Seq,
    pub snd_nxt: Seq,
    pub snd_wnd: u32,
    pub iss: Seq,
    pub rcv_nxt: Seq,
    pub rcv_wnd: u32,
    pub irs: Seq,
    pub retransmit_queue: VecDeque<SentSegment>,
    pub out_of_order: BTreeMap<u32, Vec<u8>>,
    pub send_buffer: VecDeque<u8>,
    pub receive_buffer: VecDeque<u8>,
    pub cwnd: u32,
    pub ssthresh: u32,
    pub dup_ack_count: u8,
    pub srtt: Option<Duration>,
    pub rttvar: Duration,
    pub rto: Duration,
    pub time_wait_expiry: Option<Instant>,
    pub mss: u16,
    pub peer_mss: u16,
    pub peer_wscale: Option<u8>,
    pub our_wscale: Option<u8>,
    pub sack_permitted: bool,
    pub ts_enabled: bool,
    pub last_peer_tsval: u32,
    pub base_time: Instant,
    pub fin_requested: bool,
    pub sack_scoreboard: Vec<(Seq, Seq)>,
    pub in_recovery_episode: bool,
    pub recovery_point: Seq,
    pub high_ack: Seq,
    pub pipe: u32,
    pub rescue_rx: Option<Seq>,
    pub bbr_state: BbrState,
    pub bbr_pacing_gain: f64,
    pub bbr_cwnd_gain: f64,
    pub bbr_max_bw: f64,
    pub bbr_min_rtt: Duration,
    pub bbr_min_rtt_stamp: Instant,
    pub bbr_cycle_idx: usize,
    pub bbr_cycle_stamp: Instant,
    pub bbr_delivered: u64,
    pub bbr_delivered_stamp: Instant,
    pub bbr_full_bw: f64,
    pub bbr_full_bw_count: usize,
    pub bbr_next_pacing_time: Instant,
}
pub struct Stack {
    pub mac: MacAddress,
    pub ip: Ipv4Address,
    pub gateway_ip: Ipv4Address,
    pub arp_cache: HashMap<Ipv4Address, MacAddress>,
    pub connections: conntable::ConnTable<SocketTuple, TcpConnection>,
    pub listening_ports: HashMap<u16, TcpState>,
    pub udp_inbox: HashMap<UdpTuple, VecDeque<Vec<u8>>>,
    pub tx_queue: VecDeque<Vec<u8>>,
    isn_counter: u32,
}
impl Stack {
    pub fn new(mac: MacAddress, ip: Ipv4Address, gateway_ip: Ipv4Address) -> Self {
        Self {
            mac, ip, gateway_ip,
            arp_cache: HashMap::new(),
            connections: conntable::ConnTable::new(conntable::DEFAULT_MAX_CONNS),
            listening_ports: HashMap::new(),
            udp_inbox: HashMap::new(),
            tx_queue: VecDeque::new(),
            isn_counter: 21245643,
        }
    }
    pub fn listen(&mut self, port: u16) {
        self.listening_ports.insert(port, TcpState::Listen);
    }
    fn generate_isn(&mut self) -> Seq {
        self.isn_counter = self.isn_counter.wrapping_add(1043);
        Seq(self.isn_counter)
    }
    pub fn udp_send(&mut self, remote_ip: Ipv4Address, remote_port: u16, local_port: u16, payload: &[u8]) {
        let length = (8 + payload.len()) as u16;
        let mut udp_seg = vec![0u8; length as usize];
        udp_seg[0..2].copy_from_slice(&local_port.to_be_bytes());
        udp_seg[2..4].copy_from_slice(&remote_port.to_be_bytes());
        udp_seg[4..6].copy_from_slice(&length.to_be_bytes());
        udp_seg[6..8].copy_from_slice(&[0, 0]);
        udp_seg[8..].copy_from_slice(payload);
        let csum = udp_checksum(self.ip, remote_ip, &udp_seg);
        udp_seg[6..8].copy_from_slice(&csum.to_be_bytes());
        let frame = self.encapsulate_ipv4_udp(self.ip, remote_ip, udp_seg);
        self.tx_queue.push_back(frame);
    }
    pub fn udp_recv(&mut self, local_port: u16) -> Option<(Ipv4Address, u16, Vec<u8>)> {
        for (tuple, queue) in self.udp_inbox.iter_mut() {
            if tuple.local_port == local_port {
                if let Some(pkt) = queue.pop_front() {
                    return Some((tuple.remote_ip, tuple.remote_port, pkt));
                }
            }
        }
        None
    }
    pub fn tcp_connect(&mut self, remote_ip: Ipv4Address, remote_port: u16, local_port: u16, now: Instant) -> SocketTuple {
        let tuple = SocketTuple {
            local_ip: self.ip,
            local_port,
            remote_ip,
            remote_port,
        };
        let iss = self.generate_isn();
        let mut conn = TcpConnection::new(tuple, TcpState::SynSent, self.mac, iss, Seq(0), now);
        conn.snd_nxt = iss.wrapping_add(1);
        let tsval = conn.current_ts(now);
        let hdr = TcpHeader {
            src_port: local_port,
            dst_port: remote_port,
            seq: iss,
            ack: Seq(0),
            data_offset: 5,
            flags: TcpFlags::SYN,
            window: 65535,
            checksum: 0,
            options: vec![
                TcpOption::Mss(1460),
                TcpOption::WindowScale(7),
                TcpOption::SackPermitted,
                TcpOption::Timestamp { tsval, tsecr: 0 }
            ],
        };
        let mut seg = hdr.serialize();
        let csum = tcp_checksum(self.ip, remote_ip, &seg);
        seg[16..18].copy_from_slice(&csum.to_be_bytes());
        let frame = self.encapsulate_ipv4_tcp(self.ip, remote_ip, seg.clone());
        self.tx_queue.push_back(frame);
        conn.retransmit_queue.push_back(SentSegment {
            seq: iss, seq_len: 1, data: seg, sent_at: now, retransmit_count: 0, sacked: false, lost: false, delivered_at_send: 0,
        });
        self.connections.insert(tuple, conn);
        tuple
    }
    pub fn tcp_send(&mut self, tuple: SocketTuple, data: &[u8]) {
        if let Some(conn) = self.connections.get_mut_by_key(&tuple) {
            conn.send_buffer.extend(data);
        }
    }
    pub fn tcp_recv(&mut self, tuple: SocketTuple) -> Vec<u8> {
        if let Some(conn) = self.connections.get_mut_by_key(&tuple) {
            conn.receive_buffer.drain(..).collect()
        } else {
            Vec::new()
        }
    }
    pub fn tcp_close(&mut self, tuple: SocketTuple, now: Instant) {
        if let Some(conn) = self.connections.get_mut_by_key(&tuple) {
            conn.fin_requested = true;
            conn.flush_send_buffer(&mut self.tx_queue, now);
        }
    }
    pub fn active_connections(&self) -> Vec<SocketTuple> {
        self.connections.keys().copied().collect()
    }
    pub fn connection_state(&self, tuple: SocketTuple) -> Option<TcpState> {
        self.connections.get_by_key(&tuple).map(|c| c.state)
    }
    pub fn flush(&mut self, now: Instant) {
        let tuples: Vec<_> = self.connections.keys().copied().collect();
        for tuple in tuples {
            if let Some(conn) = self.connections.get_mut_by_key(&tuple) {
                conn.flush_send_buffer(&mut self.tx_queue, now);
            }
        }
    }
    pub fn on_packet(&mut self, frame: &[u8], now: Instant) {
        let __probe_start = probes::stage_begin(probes::StageId::TotalPacket);
        let _guard = scopeguard_end(probes::StageId::TotalPacket, __probe_start);
        if frame.len() < 14 { return; }
        let dst_mac = MacAddress(frame[0..6].try_into().unwrap());
        let src_mac = MacAddress(frame[6..12].try_into().unwrap());
        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        if dst_mac != self.mac && dst_mac != MacAddress([0xff; 6]) {
            return;
        }
        match ethertype {
            0x0806 => self.handle_arp(frame, src_mac),
            0x0800 => self.handle_ipv4(frame, src_mac, now),
            _ => {}
        }
    }
    pub fn on_tick(&mut self, now: Instant) {
        let tuples: Vec<_> = self.connections.keys().copied().collect();
        for tuple in tuples {
            if let Some(conn) = self.connections.get_mut_by_key(&tuple) {
                conn.handle_timers(now, &mut self.tx_queue);
                conn.flush_send_buffer(&mut self.tx_queue, now);
                if conn.state == TcpState::Closed {
                    self.connections.remove_by_key(&tuple);
                }
            }
        }
    }
    fn handle_arp(&mut self, frame: &[u8], src_mac: MacAddress) {
        if frame.len() < 42 { return; }
        let target_ip = Ipv4Address(frame[38..42].try_into().unwrap());
        if target_ip != self.ip { return; }
        let sender_ip = Ipv4Address(frame[28..32].try_into().unwrap());
        self.arp_cache.insert(sender_ip, src_mac);
        let mut reply = vec![0u8; 42];
        reply[0..6].copy_from_slice(&src_mac.0);
        reply[6..12].copy_from_slice(&self.mac.0);
        reply[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
        reply[14..16].copy_from_slice(&1u16.to_be_bytes());
        reply[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
        reply[18] = 6;
        reply[19] = 4;
        reply[20..22].copy_from_slice(&2u16.to_be_bytes());
        reply[22..28].copy_from_slice(&self.mac.0);
        reply[28..32].copy_from_slice(&self.ip.0);
        reply[32..38].copy_from_slice(&src_mac.0);
        reply[38..42].copy_from_slice(&sender_ip.0);
        self.tx_queue.push_back(reply);
    }
    fn handle_ipv4(&mut self, frame: &[u8], src_mac: MacAddress, now: Instant) {
        let __probe_start = probes::stage_begin(probes::StageId::L3Parse);
        let _guard = scopeguard_end(probes::StageId::L3Parse, __probe_start);
        if frame.len() < 34 { return; }
        let ihl = (frame[14] & 0x0F) as usize * 4;
        let total_len = u16::from_be_bytes([frame[16], frame[17]]) as usize;
        let protocol = frame[14 + 9];
        let src_ip = Ipv4Address(frame[14 + 12..14 + 16].try_into().unwrap());
        let dst_ip = Ipv4Address(frame[14 + 16..14 + 20].try_into().unwrap());
        if dst_ip != self.ip { return; }
        if checksum(&frame[14..14 + ihl]) != 0 { return; }
        self.arp_cache.insert(src_ip, src_mac);
        let transport_start = 14 + ihl;
        let transport_end = 14 + total_len;
        if frame.len() < transport_end { return; }
        if protocol == 6 {
            self.handle_tcp(src_ip, dst_ip, &frame[transport_start..transport_end], now);
        } else if protocol == 17 {
            self.handle_udp(src_ip, dst_ip, &frame[transport_start..transport_end]);
        }
    }
    fn handle_udp(&mut self, src_ip: Ipv4Address, dst_ip: Ipv4Address, udp_segment: &[u8]) {
        let (hdr, payload) = match UdpHeader::parse(udp_segment) {
            Some(res) => res,
            None => return,
        };
        if hdr.checksum != 0 && udp_checksum(src_ip, dst_ip, udp_segment) != 0 {
            return;
        }
        let tuple = UdpTuple {
            local_ip: dst_ip,
            local_port: hdr.dst_port,
            remote_ip: src_ip,
            remote_port: hdr.src_port,
        };
        self.udp_inbox.entry(tuple).or_default().push_back(payload.to_vec());
    }
    fn handle_tcp(&mut self, src_ip: Ipv4Address, dst_ip: Ipv4Address, tcp_segment: &[u8], now: Instant) {
        let __probe_start = probes::stage_begin(probes::StageId::L4Demux);
        let _guard = scopeguard_end(probes::StageId::L4Demux, __probe_start);
        if tcp_checksum(src_ip, dst_ip, tcp_segment) != 0 { return; }
        let (hdr, payload) = match TcpHeader::parse(tcp_segment) {
            Some(res) => res,
            None => return,
        };
        let tuple = SocketTuple {
            local_ip: dst_ip, local_port: hdr.dst_port,
            remote_ip: src_ip, remote_port: hdr.src_port,
        };
        if let Some(conn) = self.connections.get_mut_by_key(&tuple) {
            conn.process_segment(hdr, payload, &mut self.tx_queue, now);
        } else if let Some(&TcpState::Listen) = self.listening_ports.get(&hdr.dst_port) {
            if hdr.flags.contains(TcpFlags::SYN) && !hdr.flags.contains(TcpFlags::ACK) {
                let iss = self.generate_isn();
                let irs = hdr.seq;
                let mut conn = TcpConnection::new(tuple, TcpState::SynReceived, self.mac, iss, irs, now);
                conn.snd_nxt = iss.wrapping_add(1);
                conn.rcv_nxt = irs.wrapping_add(1);
                conn.negotiate_options(&hdr);
                conn.send_syn_ack(&mut self.tx_queue, now);
                self.connections.insert(tuple, conn);
            }
        } else {
            self.send_rst(dst_ip, src_ip, hdr);
        }
    }
    pub fn send_arp_request(&mut self, target_ip: Ipv4Address) {
        let mut req = vec![0u8; 42];
        req[0..6].copy_from_slice(&[0xff; 6]);
        req[6..12].copy_from_slice(&self.mac.0);
        req[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
        req[14..16].copy_from_slice(&1u16.to_be_bytes());
        req[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
        req[18] = 6;
        req[19] = 4;
        req[20..22].copy_from_slice(&1u16.to_be_bytes());
        req[22..28].copy_from_slice(&self.mac.0);
        req[28..32].copy_from_slice(&self.ip.0);
        req[32..38].copy_from_slice(&[0x00; 6]);
        req[38..42].copy_from_slice(&target_ip.0);
        self.tx_queue.push_back(req);
    }
    pub fn is_same_subnet(&self, other_ip: Ipv4Address) -> bool {
        self.ip.0[0] == other_ip.0[0]
            && self.ip.0[1] == other_ip.0[1]
            && self.ip.0[2] == other_ip.0[2]
    }
    pub fn resolve_and_populate_dst_mac(&mut self, packet: &mut [u8]) {
        if packet.len() >= 34 {
            let ethertype = u16::from_be_bytes([packet[12], packet[13]]);
            if ethertype == 0x0800 {
                let dst_ip = Ipv4Address(packet[30..34].try_into().unwrap());
                let next_hop_ip = if self.is_same_subnet(dst_ip) {
                    dst_ip
                } else {
                    self.gateway_ip
                };
                if let Some(mac) = self.arp_cache.get(&next_hop_ip) {
                    packet[0..6].copy_from_slice(&mac.0);
                } else {
                    self.send_arp_request(next_hop_ip);
                }
            }
        }
    }
    fn send_rst(&mut self, local_ip: Ipv4Address, remote_ip: Ipv4Address, bad_hdr: TcpHeader) {
        if bad_hdr.flags.contains(TcpFlags::RST) { return; }
        let rst_seq = if bad_hdr.flags.contains(TcpFlags::ACK) { bad_hdr.ack } else { Seq(0) };
        let rst_ack = if !bad_hdr.flags.contains(TcpFlags::ACK) {
            bad_hdr.seq.wrapping_add(if bad_hdr.flags.contains(TcpFlags::SYN) { 1 } else { 0 })
        } else { Seq(0) };
        let mut rst_flags = TcpFlags::RST;
        if !bad_hdr.flags.contains(TcpFlags::ACK) { rst_flags.insert(TcpFlags::ACK); }
        let reply_hdr = TcpHeader {
            src_port: bad_hdr.dst_port, dst_port: bad_hdr.src_port,
            seq: rst_seq, ack: rst_ack, data_offset: 5,
            flags: rst_flags, window: 0, checksum: 0, options: Vec::new(),
        };
        let mut reply_bytes = reply_hdr.serialize();
        let csum = tcp_checksum(local_ip, remote_ip, &reply_bytes);
        reply_bytes[16..18].copy_from_slice(&csum.to_be_bytes());
        let frame = self.encapsulate_ipv4_tcp(local_ip, remote_ip, reply_bytes);
        self.tx_queue.push_back(frame);
    }
    fn encapsulate_ipv4_tcp(&mut self, src_ip: Ipv4Address, dst_ip: Ipv4Address, tcp_bytes: Vec<u8>) -> Vec<u8> {
        let mut frame = vec![0u8; 14 + 20 + tcp_bytes.len()];
        let dst_mac = self.arp_cache.get(&dst_ip).cloned().unwrap_or(MacAddress([0xff; 6]));
        frame[0..6].copy_from_slice(&dst_mac.0);
        frame[6..12].copy_from_slice(&self.mac.0);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[14] = 0x45;
        frame[15] = 0x00;
        let total_len = (20 + tcp_bytes.len()) as u16;
        frame[16..18].copy_from_slice(&total_len.to_be_bytes());
        frame[18..20].copy_from_slice(&0u16.to_be_bytes());
        frame[20..22].copy_from_slice(&0x4000u16.to_be_bytes());
        frame[22] = 64;
        frame[23] = 6;
        frame[24..26].copy_from_slice(&0u16.to_be_bytes());
        frame[26..30].copy_from_slice(&src_ip.0);
        frame[30..34].copy_from_slice(&dst_ip.0);
        let ip_csum = checksum(&frame[14..34]);
        frame[24..26].copy_from_slice(&ip_csum.to_be_bytes());
        frame[34..].copy_from_slice(&tcp_bytes);
        frame
    }
    fn encapsulate_ipv4_udp(&mut self, src_ip: Ipv4Address, dst_ip: Ipv4Address, udp_bytes: Vec<u8>) -> Vec<u8> {
        let mut frame = vec![0u8; 14 + 20 + udp_bytes.len()];
        let dst_mac = self.arp_cache.get(&dst_ip).cloned().unwrap_or(MacAddress([0xff; 6]));
        frame[0..6].copy_from_slice(&dst_mac.0);
        frame[6..12].copy_from_slice(&self.mac.0);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[14] = 0x45;
        frame[15] = 0x00;
        let total_len = (20 + udp_bytes.len()) as u16;
        frame[16..18].copy_from_slice(&total_len.to_be_bytes());
        frame[18..20].copy_from_slice(&0u16.to_be_bytes());
        frame[20..22].copy_from_slice(&0x0000u16.to_be_bytes());
        frame[22] = 64;
        frame[23] = 17;
        frame[24..26].copy_from_slice(&0u16.to_be_bytes());
        frame[26..30].copy_from_slice(&src_ip.0);
        frame[30..34].copy_from_slice(&dst_ip.0);
        let ip_csum = checksum(&frame[14..34]);
        frame[24..26].copy_from_slice(&ip_csum.to_be_bytes());
        frame[34..].copy_from_slice(&udp_bytes);
        frame
    }
}
impl TcpConnection {
    pub fn new(tuple: SocketTuple, state: TcpState, local_mac: MacAddress, iss: Seq, irs: Seq, now: Instant) -> Self {
        Self {
            tuple, state, local_mac,
            snd_una: iss, snd_nxt: iss, snd_wnd: 65535, iss,
            rcv_nxt: irs, rcv_wnd: 65535, irs,
            retransmit_queue: VecDeque::new(),
            out_of_order: BTreeMap::new(),
            send_buffer: VecDeque::new(),
            receive_buffer: VecDeque::new(),
            cwnd: 5840, ssthresh: 65535, dup_ack_count: 0,
            srtt: None, rttvar: Duration::from_millis(100),
            rto: Duration::from_millis(200),
            time_wait_expiry: None,
            mss: 1460,
            peer_mss: 536,
            peer_wscale: None,
            our_wscale: Some(7),
            sack_permitted: false,
            ts_enabled: false,
            last_peer_tsval: 0,
            base_time: now,
            fin_requested: false,
            sack_scoreboard: Vec::new(),
            in_recovery_episode: false,
            recovery_point: iss,
            high_ack: iss,
            pipe: 0,
            rescue_rx: None,
            bbr_state: BbrState::Startup,
            bbr_pacing_gain: 2.89,
            bbr_cwnd_gain: 2.89,
            bbr_max_bw: 0.1,
            bbr_min_rtt: Duration::from_millis(200),
            bbr_min_rtt_stamp: now,
            bbr_cycle_idx: 0,
            bbr_cycle_stamp: now,
            bbr_delivered: 0,
            bbr_delivered_stamp: now,
            bbr_full_bw: 0.0,
            bbr_full_bw_count: 0,
            bbr_next_pacing_time: now,
        }
    }
    pub fn current_ts(&self, now: Instant) -> u32 {
        (now.duration_since(self.base_time).as_millis() as u32).wrapping_add(1000)
    }
    pub fn generate_sack_blocks(&self) -> Vec<(Seq, Seq)> {
        let mut blocks = Vec::new();
        if self.out_of_order.is_empty() { return blocks; }
        let mut start = None;
        let mut end = None;
        for (&seq, payload) in &self.out_of_order {
            let block_start = Seq(seq);
            let block_end = block_start.wrapping_add(payload.len() as u32);
            match (start, end) {
                (None, None) => {
                    start = Some(block_start);
                    end = Some(block_end);
                }
                (Some(_s), Some(e)) if e == block_start => {
                    end = Some(block_end);
                }
                (Some(s), Some(e)) => {
                    blocks.push((s, e));
                    start = Some(block_start);
                    end = Some(block_end);
                    if blocks.len() == 3 { break; }
                }
                _ => {}
            }
        }
        if let (Some(s), Some(e)) = (start, end) {
            if blocks.len() < 3 {
                blocks.push((s, e));
            }
        }
        blocks
    }
    pub fn negotiate_options(&mut self, hdr: &TcpHeader) {
        let mut peer_wscale_found = false;
        for opt in &hdr.options {
            match opt {
                TcpOption::Mss(mss) => { self.peer_mss = *mss; }
                TcpOption::WindowScale(scale) => {
                    self.peer_wscale = Some(*scale);
                    peer_wscale_found = true;
                }
                TcpOption::SackPermitted => { self.sack_permitted = true; }
                TcpOption::Timestamp { tsval, .. } => {
                    self.ts_enabled = true;
                    self.last_peer_tsval = *tsval;
                }
                _ => {}
            }
        }
        if !peer_wscale_found && hdr.flags.contains(TcpFlags::SYN) {
            self.our_wscale = None;
        }
    }
    fn send_syn_ack(&mut self, tx_queue: &mut VecDeque<Vec<u8>>, now: Instant) {
        let mut options = vec![TcpOption::Mss(self.mss)];
        if let Some(scale) = self.our_wscale {
            options.push(TcpOption::WindowScale(scale));
        }
        if self.sack_permitted {
            options.push(TcpOption::SackPermitted);
        }
        if self.ts_enabled {
            let tsval = self.current_ts(now);
            options.push(TcpOption::Timestamp { tsval, tsecr: self.last_peer_tsval });
        }
        let hdr = TcpHeader {
            src_port: self.tuple.local_port, dst_port: self.tuple.remote_port,
            seq: self.iss, ack: self.rcv_nxt, data_offset: 5,
            flags: TcpFlags::SYN | TcpFlags::ACK, window: (self.rcv_wnd >> self.our_wscale.unwrap_or(0)) as u16, checksum: 0,
            options,
        };
        let mut seg = hdr.serialize();
        let csum = tcp_checksum(self.tuple.local_ip, self.tuple.remote_ip, &seg);
        seg[16..18].copy_from_slice(&csum.to_be_bytes());
        tx_queue.push_back(self.encapsulate_ipv4_tcp(seg.clone()));
        self.retransmit_queue.push_back(SentSegment {
            seq: self.iss, seq_len: 1, data: seg, sent_at: now, retransmit_count: 0, sacked: false, lost: false, delivered_at_send: self.bbr_delivered,
        });
    }
    fn send_fin(&mut self, tx_queue: &mut VecDeque<Vec<u8>>, now: Instant) {
        let mut options = Vec::new();
        if self.ts_enabled {
            let tsval = self.current_ts(now);
            options.push(TcpOption::Timestamp { tsval, tsecr: self.last_peer_tsval });
        }
        let hdr = TcpHeader {
            src_port: self.tuple.local_port, dst_port: self.tuple.remote_port,
            seq: self.snd_nxt, ack: self.rcv_nxt, data_offset: 5,
            flags: TcpFlags::FIN | TcpFlags::ACK, window: (self.rcv_wnd >> self.our_wscale.unwrap_or(0)) as u16, checksum: 0,
            options,
        };
        let mut seg = hdr.serialize();
        let csum = tcp_checksum(self.tuple.local_ip, self.tuple.remote_ip, &seg);
        seg[16..18].copy_from_slice(&csum.to_be_bytes());
        tx_queue.push_back(self.encapsulate_ipv4_tcp(seg.clone()));
        self.retransmit_queue.push_back(SentSegment {
            seq: self.snd_nxt, seq_len: 1, data: seg, sent_at: now, retransmit_count: 0, sacked: false, lost: false, delivered_at_send: self.bbr_delivered,
        });
        self.snd_nxt = self.snd_nxt.wrapping_add(1);
    }
    fn send_empty_ack(&mut self, tx_queue: &mut VecDeque<Vec<u8>>, now: Instant) {
        let mut options = Vec::new();
        if self.ts_enabled {
            let tsval = self.current_ts(now);
            options.push(TcpOption::Timestamp { tsval, tsecr: self.last_peer_tsval });
        }
        if self.sack_permitted {
            let blocks = self.generate_sack_blocks();
            if !blocks.is_empty() {
                options.push(TcpOption::Sack(blocks));
            }
        }
        let hdr = TcpHeader {
            src_port: self.tuple.local_port, dst_port: self.tuple.remote_port,
            seq: self.snd_nxt, ack: self.rcv_nxt, data_offset: 5,
            flags: TcpFlags::ACK, window: (self.rcv_wnd >> self.our_wscale.unwrap_or(0)) as u16, checksum: 0,
            options,
        };
        let mut seg = hdr.serialize();
        let csum = tcp_checksum(self.tuple.local_ip, self.tuple.remote_ip, &seg);
        seg[16..18].copy_from_slice(&csum.to_be_bytes());
        tx_queue.push_back(self.encapsulate_ipv4_tcp(seg));
    }
    fn drain_retransmit_queue(&mut self, ack: Seq, now: Instant) {
        let mut rtt_sample = None;
        let mut delivered_bytes = 0u64;
        let mut prior_delivered = self.bbr_delivered;
        let mut packet_send_time = now;
        while let Some(front) = self.retransmit_queue.front() {
            let seg_end = front.seq.wrapping_add(front.seq_len);
            if seg_end.lte(ack) {
                let seg = self.retransmit_queue.pop_front().unwrap();
                delivered_bytes += seg.seq_len as u64;
                prior_delivered = seg.delivered_at_send;
                packet_send_time = seg.sent_at;
                if seg.retransmit_count == 0 && rtt_sample.is_none() {
                    rtt_sample = Some(now.duration_since(seg.sent_at));
                }
            } else {
                break;
            }
        }
        self.bbr_delivered += delivered_bytes;
        self.bbr_delivered_stamp = now;
        if let Some(rtt) = rtt_sample {
            self.update_rtt(rtt);
            self.update_bbr_model(delivered_bytes, prior_delivered, packet_send_time, rtt, now);
        }
    }
    fn compute_rto(&self) -> Duration {
        match self.srtt {
            None => Duration::from_millis(200),
            Some(srtt) => (srtt + self.rttvar * 4).max(Duration::from_millis(100)),
        }
    }
    fn update_rtt(&mut self, rtt: Duration) {
        match self.srtt {
            None => {
                self.srtt = Some(rtt);
                self.rttvar = rtt / 2;
            }
            Some(srtt) => {
                let diff = if rtt > srtt { rtt - srtt } else { srtt - rtt };
                self.rttvar = self.rttvar.mul_f64(0.75) + diff.mul_f64(0.25);
                self.srtt = Some(srtt.mul_f64(0.875) + rtt.mul_f64(0.125));
            }
        }
        self.rto = self.compute_rto();
    }
    fn update_bbr_model(&mut self, delivered: u64, prior_delivered: u64, sent_time: Instant, rtt: Duration, now: Instant) {
        let __probe_start = probes::stage_begin(probes::StageId::BbrUpdate);
        let _guard = scopeguard_end(probes::StageId::BbrUpdate, __probe_start);
        if rtt < self.bbr_min_rtt || now.duration_since(self.bbr_min_rtt_stamp) > Duration::from_secs(10) {
            self.bbr_min_rtt = rtt;
            self.bbr_min_rtt_stamp = now;
        }
        let delivery_interval = now.duration_since(sent_time);
        if delivery_interval.as_micros() > 0 && delivered > 0 {
            let sample_bw = (self.bbr_delivered - prior_delivered) as f64 / delivery_interval.as_micros() as f64;
            if sample_bw > self.bbr_max_bw {
                self.bbr_max_bw = sample_bw;
            }
        }
        self.update_bbr_state_machine(now);
    }
    fn update_bbr_state_machine(&mut self, now: Instant) {
        match self.bbr_state {
            BbrState::Startup => {
                if self.bbr_max_bw > self.bbr_full_bw * 1.25 {
                    self.bbr_full_bw = self.bbr_max_bw;
                    self.bbr_full_bw_count = 0;
                } else {
                    self.bbr_full_bw_count += 1;
                    if self.bbr_full_bw_count >= 3 {
                        self.bbr_state = BbrState::Drain;
                        self.bbr_pacing_gain = 1.0 / 2.89;
                        self.bbr_cwnd_gain = 2.89;
                    }
                }
            }
            BbrState::Drain => {
                let pipe = self.calculate_pipe();
                let target_cwnd = (self.bbr_max_bw * self.bbr_min_rtt.as_micros() as f64) as u32;
                if pipe <= target_cwnd {
                    self.bbr_state = BbrState::ProbeBw;
                    self.bbr_pacing_gain = 1.25;
                    self.bbr_cwnd_gain = 2.0;
                    self.bbr_cycle_stamp = now;
                    self.bbr_cycle_idx = 0;
                }
            }
            BbrState::ProbeBw => {
                let cycle_gains = [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
                if now.duration_since(self.bbr_cycle_stamp) > self.bbr_min_rtt {
                    self.bbr_cycle_idx = (self.bbr_cycle_idx + 1) % 8;
                    self.bbr_pacing_gain = cycle_gains[self.bbr_cycle_idx];
                    self.bbr_cycle_stamp = now;
                }
            }
            BbrState::ProbeRtt => {
                if now.duration_since(self.bbr_min_rtt_stamp) > Duration::from_millis(200) {
                    self.bbr_state = BbrState::ProbeBw;
                    self.bbr_pacing_gain = 1.0;
                    self.bbr_cwnd_gain = 2.0;
                }
            }
        }
        let target_cwnd = (self.bbr_max_bw * self.bbr_min_rtt.as_micros() as f64 * self.bbr_cwnd_gain) as u32;
        self.cwnd = target_cwnd.max(4 * self.mss as u32);
    }
    pub fn update_sack_scoreboard(&mut self, blocks: &[(Seq, Seq)]) {
        let __probe_start = probes::stage_begin(probes::StageId::SackScoreboard);
        let _guard = scopeguard_end(probes::StageId::SackScoreboard, __probe_start);
        for &(start, end) in blocks {
            for seg in self.retransmit_queue.iter_mut() {
                let seg_start = seg.seq;
                let seg_end = seg.seq.wrapping_add(seg.seq_len);
                if start.lte(seg_start) && seg_end.lte(end) {
                    seg.sacked = true;
                }
            }
        }
        let mut sacked_ahead_count = 0;
        for i in (0..self.retransmit_queue.len()).rev() {
            if self.retransmit_queue[i].sacked {
                sacked_ahead_count += 1;
            } else if sacked_ahead_count >= 3 {
                self.retransmit_queue[i].lost = true;
            }
        }
        self.pipe = self.calculate_pipe();
    }
    fn calculate_pipe(&self) -> u32 {
        let mut outstanding_bytes = 0;
        for seg in &self.retransmit_queue {
            if !seg.sacked && !seg.lost {
                outstanding_bytes += seg.seq_len;
            }
        }
        outstanding_bytes
    }
    fn next_seg_to_transmit(&mut self) -> Option<(Seq, Vec<u8>)> {
        for seg in &self.retransmit_queue {
            if seg.lost && !seg.sacked && seg.retransmit_count == 0 {
                return Some((seg.seq, seg.data.clone()));
            }
        }
        for seg in &self.retransmit_queue {
            if !seg.sacked && !seg.lost && seg.retransmit_count == 0 {
                return Some((seg.seq, seg.data.clone()));
            }
        }
        if self.pipe < 3 * self.mss as u32 {
            if let Some(last_seg) = self.retransmit_queue.back() {
                if !last_seg.sacked && self.rescue_rx != Some(last_seg.seq) {
                    return Some((last_seg.seq, last_seg.data.clone()));
                }
            }
        }
        None
    }
    pub fn flush_send_buffer(&mut self, tx_queue: &mut VecDeque<Vec<u8>>, now: Instant) {
        let __probe_start = probes::stage_begin(probes::StageId::TxEnqueue);
        let _guard = scopeguard_end(probes::StageId::TxEnqueue, __probe_start);
        if now < self.bbr_next_pacing_time {
            return;
        }
        if self.in_recovery_episode {
            while self.pipe < self.cwnd {
                if let Some((seq, packet_data)) = self.next_seg_to_transmit() {
                    let mut packet = packet_data;
                    packet[16..18].copy_from_slice(&[0, 0]);
                    let csum = tcp_checksum(self.tuple.local_ip, self.tuple.remote_ip, &packet);
                    packet[16..18].copy_from_slice(&csum.to_be_bytes());
                    tx_queue.push_back(self.encapsulate_ipv4_tcp(packet));
                    if let Some(seg) = self.retransmit_queue.iter_mut().find(|s| s.seq == seq) {
                        seg.sent_at = now;
                        seg.retransmit_count += 1;
                        if self.pipe < 3 * self.mss as u32 {
                            self.rescue_rx = Some(seq);
                        }
                    }
                    self.pipe = self.calculate_pipe();
                    self.update_pacing_barrier(self.mss as usize, now);
                } else {
                    break;
                }
            }
        }
        loop {
            if self.send_buffer.is_empty() { break; }
            let in_flight = self.snd_nxt.wrapping_sub(self.snd_una);
            let window = (self.cwnd.min(self.snd_wnd)).saturating_sub(in_flight);
            if window == 0 { break; }
            let chunk_len = (window as usize).min(self.peer_mss as usize).min(self.send_buffer.len());
            if chunk_len == 0 { break; }
            let chunk: Vec<u8> = self.send_buffer.drain(..chunk_len).collect();
            let mut options = Vec::new();
            if self.ts_enabled {
                let tsval = self.current_ts(now);
                options.push(TcpOption::Timestamp { tsval, tsecr: self.last_peer_tsval });
            }
            let hdr = TcpHeader {
                src_port: self.tuple.local_port, dst_port: self.tuple.remote_port,
                seq: self.snd_nxt, ack: self.rcv_nxt, data_offset: 5,
                flags: TcpFlags::ACK | TcpFlags::PSH, window: (self.rcv_wnd >> self.our_wscale.unwrap_or(0)) as u16, checksum: 0,
                options,
            };
            let mut seg = hdr.serialize();
            seg.extend_from_slice(&chunk);
            let csum = tcp_checksum(self.tuple.local_ip, self.tuple.remote_ip, &seg);
            seg[16..18].copy_from_slice(&csum.to_be_bytes());
            tx_queue.push_back(self.encapsulate_ipv4_tcp(seg.clone()));
            self.retransmit_queue.push_back(SentSegment {
                seq: self.snd_nxt, seq_len: chunk.len() as u32,
                data: seg, sent_at: now, retransmit_count: 0, sacked: false, lost: false, delivered_at_send: self.bbr_delivered,
            });
            self.snd_nxt = self.snd_nxt.wrapping_add(chunk.len() as u32);
            self.update_pacing_barrier(chunk_len, now);
        }
        if self.send_buffer.is_empty() && self.receive_buffer.is_empty() && self.fin_requested && self.snd_una == self.snd_nxt {
            match self.state {
                TcpState::Established => {
                    self.state = TcpState::FinWait1;
                    self.send_fin(tx_queue, now);
                    self.fin_requested = false;
                }
                TcpState::CloseWait => {
                    self.state = TcpState::LastAck;
                    self.send_fin(tx_queue, now);
                    self.fin_requested = false;
                }
                _ => {}
            }
        }
    }
    fn update_pacing_barrier(&mut self, len: usize, now: Instant) {
        let pacing_delay_micros = (len as f64 / (self.bbr_max_bw * self.bbr_pacing_gain)) as u64;
        self.bbr_next_pacing_time = now + Duration::from_micros(pacing_delay_micros);
    }
    pub fn process_segment(&mut self, hdr: TcpHeader, payload: &[u8], tx_queue: &mut VecDeque<Vec<u8>>, now: Instant) {
        let __probe_start = probes::stage_begin(probes::StageId::TcpFsmStep);
        let _guard = scopeguard_end(probes::StageId::TcpFsmStep, __probe_start);
        if hdr.flags.contains(TcpFlags::RST) {
            self.state = TcpState::Closed;
            return;
        }
        for opt in &hdr.options {
            match opt {
                TcpOption::Timestamp { tsval, .. } => {
                    self.last_peer_tsval = *tsval;
                }
                TcpOption::Sack(blocks) => {
                    self.update_sack_scoreboard(blocks);
                }
                _ => {}
            }
        }
        match self.state {
            TcpState::SynSent => {
                if hdr.flags.contains(TcpFlags::SYN) && hdr.flags.contains(TcpFlags::ACK) {
                    if hdr.ack == self.snd_nxt {
                        self.irs = hdr.seq;
                        self.rcv_nxt = hdr.seq.wrapping_add(1);
                        self.snd_una = hdr.ack;
                        self.negotiate_options(&hdr);
                        if let Some(scale) = self.peer_wscale {
                            self.snd_wnd = (hdr.window as u32) << scale;
                        } else {
                            self.snd_wnd = hdr.window as u32;
                        }
                        self.retransmit_queue.clear();
                        self.state = TcpState::Established;
                        self.send_empty_ack(tx_queue, now);
                    }
                } else if hdr.flags.contains(TcpFlags::SYN) {
                    self.irs = hdr.seq;
                    self.rcv_nxt = hdr.seq.wrapping_add(1);
                    self.state = TcpState::SynReceived;
                    self.negotiate_options(&hdr);
                    self.send_syn_ack(tx_queue, now);
                }
            }
            TcpState::SynReceived => {
                if hdr.flags.contains(TcpFlags::ACK) && hdr.ack == self.snd_nxt {
                    self.state = TcpState::Established;
                    self.snd_una = hdr.ack;
                    if let Some(scale) = self.peer_wscale {
                        self.snd_wnd = (hdr.window as u32) << scale;
                    } else {
                        self.snd_wnd = hdr.window as u32;
                    }
                    self.retransmit_queue.clear();
                }
            }
            TcpState::Established | TcpState::CloseWait | TcpState::FinWait1 | TcpState::FinWait2 => {
                self.process_ack(&hdr, tx_queue, now);
                if payload.len() > 0 {
                    if hdr.seq == self.rcv_nxt {
                        self.receive_buffer.extend(payload);
                        self.rcv_nxt = self.rcv_nxt.wrapping_add(payload.len() as u32);
                        while let Some(stored) = self.out_of_order.remove(&self.rcv_nxt.0) {
                            let len = stored.len();
                            self.receive_buffer.extend(&stored);
                            self.rcv_nxt = self.rcv_nxt.wrapping_add(len as u32);
                        }
                        self.send_empty_ack(tx_queue, now);
                    } else if self.rcv_nxt.lt(hdr.seq) {
                        self.out_of_order.insert(hdr.seq.0, payload.to_vec());
                        self.send_empty_ack(tx_queue, now);
                    } else {
                        self.send_empty_ack(tx_queue, now);
                    }
                }
                if hdr.flags.contains(TcpFlags::FIN) {
                    if hdr.seq == self.rcv_nxt {
                        self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
                        match self.state {
                            TcpState::Established => {
                                self.state = TcpState::CloseWait;
                            }
                            TcpState::FinWait1 => {
                                if self.snd_una == self.snd_nxt {
                                    self.state = TcpState::TimeWait;
                                    self.time_wait_expiry = Some(now + Duration::from_secs(2));
                                } else {
                                    self.state = TcpState::Closing;
                                }
                            }
                            TcpState::FinWait2 => {
                                self.state = TcpState::TimeWait;
                                self.time_wait_expiry = Some(now + Duration::from_secs(2));
                            }
                            _ => {}
                        }
                        self.send_empty_ack(tx_queue, now);
                    }
                }
            }
            TcpState::Closing => {
                self.process_ack(&hdr, tx_queue, now);
                if self.snd_una == self.snd_nxt {
                    self.state = TcpState::TimeWait;
                    self.time_wait_expiry = Some(now + Duration::from_secs(2));
                }
            }
            TcpState::LastAck => {
                self.process_ack(&hdr, tx_queue, now);
                if hdr.flags.contains(TcpFlags::ACK) && hdr.ack == self.snd_nxt {
                    self.state = TcpState::Closed;
                }
            }
            TcpState::TimeWait => {
                if hdr.flags.contains(TcpFlags::FIN) {
                    self.send_empty_ack(tx_queue, now);
                    self.time_wait_expiry = Some(now + Duration::from_secs(2));
                }
            }
            _ => {}
        }
    }
    fn process_ack(&mut self, hdr: &TcpHeader, tx_queue: &mut VecDeque<Vec<u8>>, now: Instant) {
        if !hdr.flags.contains(TcpFlags::ACK) { return; }
        if self.snd_una.lt(hdr.ack) && hdr.ack.lte(self.snd_nxt) {
            self.dup_ack_count = 0;
            self.snd_una = hdr.ack;
            if let Some(scale) = self.peer_wscale {
                self.snd_wnd = (hdr.window as u32) << scale;
            } else {
                self.snd_wnd = hdr.window as u32;
            }
            self.drain_retransmit_queue(hdr.ack, now);
            self.rto = self.compute_rto();
            if self.in_recovery_episode {
                if self.recovery_point.lte(hdr.ack) {
                    self.in_recovery_episode = false;
                } else {
                    self.high_ack = hdr.ack;
                }
            }
            self.flush_send_buffer(tx_queue, now);
        } else if hdr.ack == self.snd_una && !self.retransmit_queue.is_empty() {
            self.dup_ack_count = self.dup_ack_count.saturating_add(1);
            if self.dup_ack_count == 3 && !self.in_recovery_episode {
                self.in_recovery_episode = true;
                self.recovery_point = self.snd_nxt;
                self.high_ack = hdr.ack;
                self.rescue_rx = None;
                if let Some(first_unacked) = self.retransmit_queue.front_mut() {
                    first_unacked.lost = true;
                }
                self.pipe = self.calculate_pipe();
                self.flush_send_buffer(tx_queue, now);
            }
        }
    }
    pub fn handle_timers(&mut self, now: Instant, tx_queue: &mut VecDeque<Vec<u8>>) {
        if let Some(expiry) = self.time_wait_expiry {
            if now >= expiry {
                self.state = TcpState::Closed;
                return;
            }
        }
        while let Some(front) = self.retransmit_queue.front() {
            if front.seq.wrapping_add(front.seq_len).lte(self.snd_una) {
                self.retransmit_queue.pop_front();
            } else {
                break;
            }
        }
        let mut to_retransmit = None;
        if let Some(seg) = self.retransmit_queue.front_mut() {
            if !seg.sacked && now.duration_since(seg.sent_at) >= self.rto {
                seg.sent_at = now;
                seg.retransmit_count += 1;
                to_retransmit = Some(seg.data.clone());
            }
        }
        if let Some(packet_data) = to_retransmit {
            self.rto = (self.rto * 2).min(Duration::from_secs(60));
            self.in_recovery_episode = false;
            if self.bbr_state != BbrState::ProbeRtt {
                self.bbr_state = BbrState::ProbeRtt;
                self.bbr_min_rtt_stamp = now;
                self.bbr_pacing_gain = 1.0;
                self.bbr_cwnd_gain = 1.0;
            }
            let mut packet = packet_data;
            packet[16..18].copy_from_slice(&[0, 0]);
            let csum = tcp_checksum(self.tuple.local_ip, self.tuple.remote_ip, &packet);
            packet[16..18].copy_from_slice(&csum.to_be_bytes());
            let frame = self.encapsulate_ipv4_tcp(packet);
            tx_queue.push_back(frame);
        }
    }
    fn encapsulate_ipv4_tcp(&mut self, tcp_bytes: Vec<u8>) -> Vec<u8> {
        let mut frame = vec![0u8; 14 + 20 + tcp_bytes.len()];
        frame[0..6].copy_from_slice(&[0x00; 6]);
        frame[6..12].copy_from_slice(&self.local_mac.0);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[14] = 0x45;
        frame[15] = 0x00;
        let total_len = (20 + tcp_bytes.len()) as u16;
        frame[16..18].copy_from_slice(&total_len.to_be_bytes());
        frame[18..20].copy_from_slice(&0u16.to_be_bytes());
        frame[20..22].copy_from_slice(&0x4000u16.to_be_bytes());
        frame[22] = 64;
        frame[23] = 6;
        frame[24..26].copy_from_slice(&0u16.to_be_bytes());
        frame[26..30].copy_from_slice(&self.tuple.local_ip.0);
        frame[30..34].copy_from_slice(&self.tuple.remote_ip.0);
        let ip_csum = checksum(&frame[14..34]);
        frame[24..26].copy_from_slice(&ip_csum.to_be_bytes());
        frame[34..].copy_from_slice(&tcp_bytes);
        frame
    }
}
#[inline(always)]
fn scopeguard_end(id: probes::StageId, start: u64) -> ProbeGuard {
    ProbeGuard { id, start }
}
struct ProbeGuard {
    id: probes::StageId,
    start: u64,
}
impl Drop for ProbeGuard {
    #[inline(always)]
    fn drop(&mut self) {
        probes::stage_end(self.id, self.start);
    }
}

