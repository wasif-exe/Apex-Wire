use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::ptr;

const PCAP_MAGIC_US: u32 = 0xa1b2c3d4;
const PCAP_MAGIC_NS: u32 = 0xa1b23c4d;
const GLOBAL_HEADER_SIZE: usize = 24;
const PACKET_HEADER_SIZE: usize = 16;

#[repr(C)]
struct PcapGlobalHeader {
    magic: u32,
    version_major: u16,
    version_minor: u16,
    thiszone: i32,
    sigfigs: u32,
    snaplen: u32,
    network: u32,
}

#[repr(C)]
struct PcapPacketHeader {
    ts_sec: u32,
    ts_frac: u32,
    incl_len: u32,
    orig_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampPrecision {
    Microseconds,
    Nanoseconds,
}

#[derive(Debug, Clone, Copy)]
pub struct PcapInfo {
    pub precision: TimestampPrecision,
    pub snaplen: u32,
    pub linktype: u32,
    pub packet_count: u64,
}

pub struct PcapPacket<'a> {
    pub timestamp_ns: u64,
    pub data: &'a [u8],
    pub orig_len: u32,
}

pub struct PcapReader {
    map: *const u8,
    file_len: usize,
    offset: usize,
    precision: TimestampPrecision,
    linktype: u32,
    snaplen: u32,
}

unsafe impl Send for PcapReader {}

impl PcapReader {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let fd = file.as_raw_fd();
        let file_len = unsafe { libc::lseek(fd, 0, libc::SEEK_END) } as usize;
        if file_len < GLOBAL_HEADER_SIZE {
            anyhow::bail!("file too small for pcap header");
        }
        unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
        let map = unsafe {
            libc::mmap(
                ptr::null_mut(),
                file_len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd,
                0,
            )
        };
        if map == libc::MAP_FAILED {
            anyhow::bail!("mmap failed");
        }
        let hdr = unsafe { &*(map as *const PcapGlobalHeader) };
        let precision = match hdr.magic {
            PCAP_MAGIC_NS => TimestampPrecision::Nanoseconds,
            PCAP_MAGIC_US => TimestampPrecision::Microseconds,
            _ => anyhow::bail!("unknown pcap magic: 0x{:08x}", hdr.magic),
        };
        Ok(Self {
            map: map as *const u8,
            file_len,
            offset: GLOBAL_HEADER_SIZE,
            precision,
            linktype: hdr.network,
            snaplen: hdr.snaplen,
        })
    }

    pub fn info(&self) -> PcapInfo {
        let mut count = 0u64;
        let mut off = GLOBAL_HEADER_SIZE;
        while off + PACKET_HEADER_SIZE <= self.file_len {
            let phdr = unsafe { &*(self.map.add(off) as *const PcapPacketHeader) };
            off += PACKET_HEADER_SIZE + phdr.incl_len as usize;
            count += 1;
        }
        PcapInfo {
            precision: self.precision,
            snaplen: self.snaplen,
            linktype: self.linktype,
            packet_count: count,
        }
    }

    pub fn linktype(&self) -> u32 {
        self.linktype
    }

    pub fn reset(&mut self) {
        self.offset = GLOBAL_HEADER_SIZE;
    }

    pub fn next_packet(&mut self) -> Option<PcapPacket<'_>> {
        if self.offset + PACKET_HEADER_SIZE > self.file_len {
            return None;
        }
        let phdr = unsafe { &*(self.map.add(self.offset) as *const PcapPacketHeader) };
        let incl = phdr.incl_len as usize;
        if self.offset + PACKET_HEADER_SIZE + incl > self.file_len {
            return None;
        }
        let ts_ns = match self.precision {
            TimestampPrecision::Nanoseconds => {
                (phdr.ts_sec as u64) * 1_000_000_000 + phdr.ts_frac as u64
            }
            TimestampPrecision::Microseconds => {
                (phdr.ts_sec as u64) * 1_000_000_000 + (phdr.ts_frac as u64) * 1_000
            }
        };
        // SAFETY: offset + PACKET_HEADER_SIZE + incl bounds-checked above within mmap region.
        let data = unsafe {
            std::slice::from_raw_parts(self.map.add(self.offset + PACKET_HEADER_SIZE), incl)
        };
        self.offset += PACKET_HEADER_SIZE + incl;
        Some(PcapPacket {
            timestamp_ns: ts_ns,
            data,
            orig_len: phdr.orig_len,
        })
    }
}

impl Drop for PcapReader {
    fn drop(&mut self) {
        // SAFETY: Unmapping previously mapped PCAP file region.
        unsafe {
            libc::munmap(self.map as *mut libc::c_void, self.file_len);
        }
    }
}

pub const MOLDUDP64_HEADER_SIZE: usize = 20;
pub const MOLDUDP64_HEARTBEAT: u16 = 0xFFFF;

#[derive(Debug, Clone, Copy)]
pub struct MoldUdp64Header {
    pub session: [u8; 10],
    pub sequence_number: u64,
    pub message_count: u16,
}

#[inline(always)]
pub fn parse_moldudp64_header(buf: &[u8]) -> Option<MoldUdp64Header> {
    if buf.len() < MOLDUDP64_HEADER_SIZE {
        return None;
    }
    let mut session = [0u8; 10];
    session.copy_from_slice(&buf[0..10]);
    let sequence_number = u64::from_be_bytes(buf[10..18].try_into().unwrap());
    let message_count = u16::from_be_bytes(buf[18..20].try_into().unwrap());
    Some(MoldUdp64Header {
        session,
        sequence_number,
        message_count,
    })
}

pub struct MoldUdp64Iter<'a> {
    buf: &'a [u8],
    offset: usize,
    remaining: u16,
}

impl<'a> MoldUdp64Iter<'a> {
    pub fn new(buf: &'a [u8], message_count: u16) -> Self {
        Self {
            buf,
            offset: 0,
            remaining: message_count,
        }
    }
}

impl<'a> Iterator for MoldUdp64Iter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        if self.offset + 2 > self.buf.len() {
            return None;
        }
        let msg_len = u16::from_be_bytes(
            self.buf[self.offset..self.offset + 2].try_into().unwrap(),
        ) as usize;
        self.offset += 2;
        if self.offset + msg_len > self.buf.len() {
            return None;
        }
        let msg = &self.buf[self.offset..self.offset + msg_len];
        self.offset += msg_len;
        self.remaining -= 1;
        Some(msg)
    }
}

#[inline(always)]
pub fn extract_udp_payload(ethernet_frame: &[u8]) -> Option<&[u8]> {
    if ethernet_frame.len() < 42 {
        return None;
    }
    let ethertype = u16::from_be_bytes([ethernet_frame[12], ethernet_frame[13]]);
    if ethertype != 0x0800 {
        return None;
    }
    let ihl = (ethernet_frame[14] & 0x0F) as usize * 4;
    if ihl < 20 {
        return None;
    }
    let protocol = ethernet_frame[14 + 9];
    if protocol != 17 {
        return None;
    }
    let transport_start = 14 + ihl;
    if transport_start + 8 > ethernet_frame.len() {
        return None;
    }
    let udp_len = u16::from_be_bytes([
        ethernet_frame[transport_start + 4],
        ethernet_frame[transport_start + 5],
    ]) as usize;
    if udp_len < 8 {
        return None;
    }
    let payload_start = transport_start + 8;
    let payload_end = payload_start + udp_len - 8;
    if payload_end > ethernet_frame.len() {
        return None;
    }
    Some(&ethernet_frame[payload_start..payload_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moldudp64_header_parse() {
        let mut buf = vec![0u8; 32];
        buf[0..10].copy_from_slice(b"NASDAQTEST");
        buf[10..18].copy_from_slice(&42u64.to_be_bytes());
        buf[18..20].copy_from_slice(&3u16.to_be_bytes());
        let hdr = parse_moldudp64_header(&buf).unwrap();
        assert_eq!(&hdr.session, b"NASDAQTEST");
        assert_eq!(hdr.sequence_number, 42);
        assert_eq!(hdr.message_count, 3);
    }

    #[test]
    fn moldudp64_iter() {
        let mut buf = vec![0u8; 20];
        buf[0..2].copy_from_slice(&4u16.to_be_bytes());
        buf[2..6].copy_from_slice(b"ABCD");
        buf[6..8].copy_from_slice(&3u16.to_be_bytes());
        buf[8..11].copy_from_slice(b"XYZ");
        let msgs: Vec<&[u8]> = MoldUdp64Iter::new(&buf, 2).collect();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], b"ABCD");
        assert_eq!(msgs[1], b"XYZ");
    }

    #[test]
    fn extract_udp_from_ethernet() {
        let mut frame = vec![0u8; 62];
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[14] = 0x45;
        frame[14 + 9] = 17;
        frame[14 + 20 + 4..14 + 20 + 6].copy_from_slice(&20u16.to_be_bytes());
        let payload = extract_udp_payload(&frame).unwrap();
        assert_eq!(payload.len(), 12);
    }
}
