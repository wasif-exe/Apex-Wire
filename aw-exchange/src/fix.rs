use aw_core::probes::{self, StageId};
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
pub const SOH: u8 = 0x01;
pub const MAX_TAGS: usize = 32;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixTagValue<'a> {
    pub tag: u32,
    pub val: &'a [u8],
}
#[derive(Debug, Clone)]
pub struct FixMessage<'a> {
    pub fields: [FixTagValue<'a>; MAX_TAGS],
    pub count: usize,
    pub msg_type: &'a [u8],
}
impl<'a> Default for FixMessage<'a> {
    #[inline(always)]
    fn default() -> Self {
        Self {
            fields: [FixTagValue { tag: 0, val: &[] }; MAX_TAGS],
            count: 0,
            msg_type: &[],
        }
    }
}
impl<'a> FixMessage<'a> {
    #[inline(always)]
    pub fn get_tag(&self, tag: u32) -> Option<&'a [u8]> {
        for i in 0..self.count {
            if self.fields[i].tag == tag {
                return Some(self.fields[i].val);
            }
        }
        None
    }
    #[inline(always)]
    pub fn get_tag_u64(&self, tag: u32) -> Option<u64> {
        let val_bytes = self.get_tag(tag)?;
        parse_ascii_u64(val_bytes)
    }
    #[inline(always)]
    pub fn get_tag_f64(&self, tag: u32) -> Option<f64> {
        let val_bytes = self.get_tag(tag)?;
        std::str::from_utf8(val_bytes).ok()?.parse::<f64>().ok()
    }
}
#[inline(always)]
pub fn parse_ascii_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut acc = 0u64;
    for &b in bytes {
        if b < b'0' || b > b'9' {
            return None;
        }
        acc = acc.wrapping_mul(10).wrapping_add((b - b'0') as u64);
    }
    Some(acc)
}
#[inline(always)]
pub fn parse_ascii_u32(bytes: &[u8]) -> Option<u32> {
    parse_ascii_u64(bytes).map(|v| v as u32)
}
#[inline(always)]
pub fn parse_fix_scalar<'a>(buf: &'a [u8], out: &mut FixMessage<'a>) -> bool {
    out.count = 0;
    let mut offset = 0;
    let len = buf.len();
    while offset < len && out.count < MAX_TAGS {
        let mut eq_idx = None;
        let mut soh_idx = None;
        let mut i = offset;
        while i < len {
            if buf[i] == b'=' && eq_idx.is_none() {
                eq_idx = Some(i);
            } else if buf[i] == SOH || buf[i] == b'|' {
                soh_idx = Some(i);
                break;
            }
            i += 1;
        }
        match (eq_idx, soh_idx) {
            (Some(eq), Some(soh)) if eq > offset && soh > eq => {
                let tag_slice = &buf[offset..eq];
                let val_slice = &buf[eq + 1..soh];
                if let Some(tag_num) = parse_ascii_u32(tag_slice) {
                    if tag_num == 35 {
                        out.msg_type = val_slice;
                    }
                    out.fields[out.count] = FixTagValue {
                        tag: tag_num,
                        val: val_slice,
                    };
                    out.count += 1;
                }
                offset = soh + 1;
            }
            _ => break,
        }
    }
    out.count > 0
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn find_soh_avx2(buf: &[u8], offset: usize) -> Option<usize> {
    let rem = buf.len().saturating_sub(offset);
    if rem < 32 {
        for i in offset..buf.len() {
            if buf[i] == SOH || buf[i] == b'|' {
                return Some(i);
            }
        }
        return None;
    }
    let ptr = buf.as_ptr().add(offset);
    let chunk = _mm256_loadu_si256(ptr as *const __m256i);
    let soh_vec = _mm256_set1_epi8(SOH as i8);
    let pipe_vec = _mm256_set1_epi8(b'|' as i8);
    let cmp_soh = _mm256_cmpeq_epi8(chunk, soh_vec);
    let cmp_pipe = _mm256_cmpeq_epi8(chunk, pipe_vec);
    let matched = _mm256_or_si256(cmp_soh, cmp_pipe);
    let mask = _mm256_movemask_epi8(matched) as u32;
    if mask != 0 {
        let idx = mask.trailing_zeros() as usize;
        Some(offset + idx)
    } else {
        None
    }
}
pub fn parse_fix<'a>(buf: &'a [u8], out: &mut FixMessage<'a>) -> bool {
    let t_begin = probes::stage_begin(StageId::FixParse);
    let ok = parse_fix_scalar(buf, out);
    probes::stage_end(StageId::FixParse, t_begin);
    ok
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_new_order_single() {
        let raw = b"8=FIX.4.4|9=122|35=D|49=SENDER|56=TARGET|34=1|52=20250101-00:00:00.000|11=ORD123|55=AAPL|54=1|38=100|44=185.50|40=2|10=000|";
        let mut fix = FixMessage::default();
        let ok = parse_fix(raw, &mut fix);
        assert!(ok);
        assert_eq!(fix.msg_type, b"D");
        assert_eq!(fix.get_tag(11), Some(&b"ORD123"[..]));
        assert_eq!(fix.get_tag(55), Some(&b"AAPL"[..]));
        assert_eq!(fix.get_tag_u64(38), Some(100));
        assert_eq!(fix.get_tag(54), Some(&b"1"[..]));
    }
}
