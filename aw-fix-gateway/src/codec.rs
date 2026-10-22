use aw_exchange::fix::{parse_fix_scalar, FixMessage, SOH};

pub type FixSeqNum = u64;

pub struct FixEncoder {
    sender: [u8; 32],
    sender_len: usize,
    target: [u8; 32],
    target_len: usize,
    version: &'static [u8],
    out_seq: FixSeqNum,
}

impl FixEncoder {
    pub fn new(sender: &str, target: &str) -> Self {
        let mut s = [0u8; 32];
        let sl = sender.len().min(32);
        s[..sl].copy_from_slice(&sender.as_bytes()[..sl]);
        let mut t = [0u8; 32];
        let tl = target.len().min(32);
        t[..tl].copy_from_slice(&target.as_bytes()[..tl]);
        Self {
            sender: s,
            sender_len: sl,
            target: t,
            target_len: tl,
            version: b"FIX.4.4",
            out_seq: 1,
        }
    }

    #[inline]
    pub fn next_seq(&mut self) -> FixSeqNum {
        let s = self.out_seq;
        self.out_seq += 1;
        s
    }

    pub fn out_seq(&self) -> FixSeqNum {
        self.out_seq
    }

    pub fn set_out_seq(&mut self, seq: FixSeqNum) {
        self.out_seq = seq;
    }

    fn write_header(&self, buf: &mut [u8], msg_type: &[u8], seq: FixSeqNum) -> usize {
        let mut pos = 0;
        pos += write_tag(buf, pos, 8, self.version);
        pos += write_tag_raw(buf, pos, b"9", b"000");
        pos += write_tag(buf, pos, 35, msg_type);
        pos += write_tag(buf, pos, 49, &self.sender[..self.sender_len]);
        pos += write_tag(buf, pos, 56, &self.target[..self.target_len]);

        let mut seq_buf = [0u8; 20];
        let seq_str = u64_to_ascii(seq, &mut seq_buf);
        pos += write_tag(buf, pos, 34, seq_str);

        pos += write_tag_raw(buf, pos, b"52", b"20250101-00:00:00.000");
        pos
    }

    pub fn encode_logon(&mut self, buf: &mut [u8], heartbeat_secs: u32) -> usize {
        let seq = self.next_seq();
        let mut pos = self.write_header(buf, b"A", seq);
        let mut hb_buf = [0u8; 10];
        let hb_str = u32_to_ascii(heartbeat_secs, &mut hb_buf);
        pos += write_tag(buf, pos, 108, hb_str);
        pos += write_tag_raw(buf, pos, b"98", b"0");
        self.patch_body_length(buf, pos);
        self.append_checksum(buf, pos)
    }

    pub fn encode_heartbeat(&mut self, buf: &mut [u8]) -> usize {
        let seq = self.next_seq();
        let pos = self.write_header(buf, b"0", seq);
        self.patch_body_length(buf, pos);
        self.append_checksum(buf, pos)
    }

    pub fn encode_logout(&mut self, buf: &mut [u8]) -> usize {
        let seq = self.next_seq();
        let pos = self.write_header(buf, b"5", seq);
        self.patch_body_length(buf, pos);
        self.append_checksum(buf, pos)
    }

    pub fn encode_new_order_single(
        &mut self,
        buf: &mut [u8],
        cl_ord_id: &[u8],
        symbol: &[u8],
        side: u8,
        qty: u64,
        price: &[u8],
        ord_type: u8,
        tif: u8,
    ) -> usize {
        let seq = self.next_seq();
        let mut pos = self.write_header(buf, b"D", seq);
        pos += write_tag(buf, pos, 11, cl_ord_id);
        pos += write_tag(buf, pos, 55, symbol);
        let side_bytes = [side];
        pos += write_tag(buf, pos, 54, &side_bytes);
        let mut qty_buf = [0u8; 20];
        let qty_str = u64_to_ascii(qty, &mut qty_buf);
        pos += write_tag(buf, pos, 38, qty_str);
        pos += write_tag(buf, pos, 44, price);
        let type_bytes = [ord_type];
        pos += write_tag(buf, pos, 40, &type_bytes);
        let tif_bytes = [tif];
        pos += write_tag(buf, pos, 59, &tif_bytes);
        self.patch_body_length(buf, pos);
        self.append_checksum(buf, pos)
    }

    fn patch_body_length(&self, buf: &mut [u8], body_end: usize) {
        let body_start = find_field_start(buf, body_end, b"35=");
        if body_start == 0 {
            return;
        }
        let body_len = body_end - body_start;
        let len_str = format_u32(body_len as u32);
        let tag9_start = find_field_start(buf, body_start, b"9=");
        if tag9_start == 0 {
            return;
        }
        let val_start = tag9_start;
        for (i, &b) in len_str.iter().enumerate() {
            if val_start + i < buf.len() {
                buf[val_start + i] = b;
            }
        }
    }

    fn append_checksum(&self, buf: &mut [u8], pos: usize) -> usize {
        let mut sum: u32 = 0;
        for &b in &buf[..pos] {
            sum = sum.wrapping_add(b as u32);
        }
        let cksum = (sum & 0xFF) as u8;
        let mut cksum_buf = [0u8; 3];
        cksum_buf[0] = b'0' + cksum / 100;
        cksum_buf[1] = b'0' + (cksum / 10) % 10;
        cksum_buf[2] = b'0' + cksum % 10;
        let n = write_tag(buf, pos, 10, &cksum_buf);
        pos + n
    }
}

pub struct FixDecoder {
    in_seq: FixSeqNum,
}

impl FixDecoder {
    pub fn new() -> Self {
        Self { in_seq: 1 }
    }

    pub fn in_seq(&self) -> FixSeqNum {
        self.in_seq
    }

    pub fn set_in_seq(&mut self, seq: FixSeqNum) {
        self.in_seq = seq;
    }

    pub fn decode<'a>(&mut self, buf: &'a [u8], out: &mut FixMessage<'a>) -> Option<usize> {
        if !parse_fix_scalar(buf, out) {
            return None;
        }
        if let Some(seq_bytes) = out.get_tag(34) {
            if let Some(seq) = aw_exchange::fix::parse_ascii_u64(seq_bytes) {
                if seq >= self.in_seq {
                    self.in_seq = seq + 1;
                }
            }
        }
        let soh_pos = buf.iter().rposition(|&b| b == SOH || b == b'|')?;
        Some(soh_pos + 1)
    }
}

#[inline]
fn write_tag(buf: &mut [u8], pos: usize, tag: u32, val: &[u8]) -> usize {
    let mut p = pos;
    let mut tag_buf = [0u8; 10];
    let tag_str = u32_to_ascii(tag, &mut tag_buf);
    for &b in tag_str {
        if p < buf.len() { buf[p] = b; }
        p += 1;
    }
    if p < buf.len() { buf[p] = b'='; }
    p += 1;
    for &b in val {
        if p < buf.len() { buf[p] = b; }
        p += 1;
    }
    if p < buf.len() { buf[p] = SOH; }
    p += 1;
    p - pos
}

#[inline]
fn write_tag_raw(buf: &mut [u8], pos: usize, tag: &[u8], val: &[u8]) -> usize {
    let mut p = pos;
    for &b in tag {
        if p < buf.len() { buf[p] = b; }
        p += 1;
    }
    if p < buf.len() { buf[p] = b'='; }
    p += 1;
    for &b in val {
        if p < buf.len() { buf[p] = b; }
        p += 1;
    }
    if p < buf.len() { buf[p] = SOH; }
    p += 1;
    p - pos
}

#[inline]
fn find_field_start(buf: &[u8], end: usize, prefix: &[u8]) -> usize {
    if end < prefix.len() + 1 {
        return 0;
    }
    for i in 0..end - prefix.len() {
        if &buf[i..i + prefix.len()] == prefix {
            return i + prefix.len();
        }
    }
    0
}

#[inline]
fn u64_to_ascii(mut v: u64, buf: &mut [u8; 20]) -> &[u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut pos = 20;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[pos..]
}

#[inline]
fn u32_to_ascii(v: u32, buf: &mut [u8; 10]) -> &[u8] {
    let mut tmp = [0u8; 20];
    let s = u64_to_ascii(v as u64, &mut tmp);
    let len = s.len().min(10);
    buf[..len].copy_from_slice(&s[..len]);
    &buf[..len]
}

#[inline]
fn format_u32(v: u32) -> [u8; 3] {
    [
        b'0' + (v / 100) as u8,
        b'0' + ((v / 10) % 10) as u8,
        b'0' + (v % 10) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_logon_roundtrip() {
        let mut enc = FixEncoder::new("SENDER", "TARGET");
        let mut buf = [0u8; 256];
        let n = enc.encode_logon(&mut buf, 30);
        assert!(n > 0);
        assert!(n < 256);
        let msg = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(msg.contains("8=FIX.4.4"));
        assert!(msg.contains("35=A"));
        assert!(msg.contains("49=SENDER"));
        assert!(msg.contains("56=TARGET"));
        assert!(msg.contains("108=30"));
    }

    #[test]
    fn encode_new_order() {
        let mut enc = FixEncoder::new("SENDER", "TARGET");
        let mut buf = [0u8; 512];
        let n = enc.encode_new_order_single(
            &mut buf,
            b"ORD001",
            b"AAPL",
            b'1',
            100,
            b"185.50",
            b'2',
            b'0',
        );
        assert!(n > 0);
        let msg = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(msg.contains("35=D"));
        assert!(msg.contains("11=ORD001"));
        assert!(msg.contains("55=AAPL"));
        assert!(msg.contains("38=100"));
        assert!(msg.contains("44=185.50"));
    }

    #[test]
    fn sequence_increments() {
        let mut enc = FixEncoder::new("S", "T");
        assert_eq!(enc.out_seq(), 1);
        let mut buf = [0u8; 256];
        enc.encode_heartbeat(&mut buf);
        assert_eq!(enc.out_seq(), 2);
        enc.encode_heartbeat(&mut buf);
        assert_eq!(enc.out_seq(), 3);
    }

    #[test]
    fn decoder_tracks_seq() {
        let mut dec = FixDecoder::new();
        let raw = b"8=FIX.4.4|9=50|35=0|49=S|56=T|34=5|52=20250101-00:00:00|10=000|";
        let mut msg = FixMessage::default();
        let consumed = dec.decode(raw, &mut msg);
        assert!(consumed.is_some());
        assert_eq!(dec.in_seq(), 6);
    }
}
