use crate::order::{EventKind, MarketEvent, Side};
use aw_core::probes::{self, StageId};
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ItchAddOrder {
    pub msg_type: u8,
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: [u8; 6],
    pub order_reference_number: u64,
    pub buy_sell_indicator: u8,
    pub shares: u32,
    pub stock: [u8; 8],
    pub price: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ItchOrderExecuted {
    pub msg_type: u8,
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: [u8; 6],
    pub order_reference_number: u64,
    pub executed_shares: u32,
    pub match_number: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ItchOrderCancel {
    pub msg_type: u8,
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: [u8; 6],
    pub order_reference_number: u64,
    pub canceled_shares: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ItchOrderDelete {
    pub msg_type: u8,
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: [u8; 6],
    pub order_reference_number: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ItchOrderReplace {
    pub msg_type: u8,
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: [u8; 6],
    pub original_order_reference_number: u64,
    pub new_order_reference_number: u64,
    pub shares: u32,
    pub price: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ItchTrade {
    pub msg_type: u8,
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: [u8; 6],
    pub order_reference_number: u64,
    pub buy_sell_indicator: u8,
    pub shares: u32,
    pub stock: [u8; 8],
    pub price: u32,
    pub match_number: u64,
}

#[inline(always)]
pub fn parse_timestamp_48(ts: &[u8; 6]) -> u64 {
    let mut buf = [0u8; 8];
    buf[2..8].copy_from_slice(ts);
    u64::from_be_bytes(buf)
}

#[cold]
#[inline(never)]
fn parse_non_add(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.is_empty() {
        return false;
    }
    let ptr = buf.as_ptr();
    match buf[0] {
        b'E' => {
            if buf.len() < 31 {
                return false;
            }
            // SAFETY: buf.len() >= 31, all pointer reads within bounds.
            unsafe {
                event.kind = EventKind::OrderExecuted;
                event.stock_locate = u16::from_be((ptr.add(1) as *const u16).read_unaligned());
                event.timestamp_ns = u64::from_be((ptr.add(3) as *const u64).read_unaligned()) & 0x0000_FFFF_FFFF_FFFF;
                event.order_id = u64::from_be((ptr.add(11) as *const u64).read_unaligned());
                event.shares = u32::from_be((ptr.add(19) as *const u32).read_unaligned());
                event.match_number = u64::from_be((ptr.add(23) as *const u64).read_unaligned());
            }
            true
        }
        b'X' => {
            if buf.len() < 23 {
                return false;
            }
            // SAFETY: buf.len() >= 23, all pointer reads within bounds.
            unsafe {
                event.kind = EventKind::OrderCancel;
                event.stock_locate = u16::from_be((ptr.add(1) as *const u16).read_unaligned());
                event.timestamp_ns = u64::from_be((ptr.add(3) as *const u64).read_unaligned()) & 0x0000_FFFF_FFFF_FFFF;
                event.order_id = u64::from_be((ptr.add(11) as *const u64).read_unaligned());
                event.shares = u32::from_be((ptr.add(19) as *const u32).read_unaligned());
            }
            true
        }
        b'D' => {
            if buf.len() < 19 {
                return false;
            }
            // SAFETY: buf.len() >= 19, all pointer reads within bounds.
            unsafe {
                event.kind = EventKind::OrderDelete;
                event.stock_locate = u16::from_be((ptr.add(1) as *const u16).read_unaligned());
                event.timestamp_ns = u64::from_be((ptr.add(3) as *const u64).read_unaligned()) & 0x0000_FFFF_FFFF_FFFF;
                event.order_id = u64::from_be((ptr.add(11) as *const u64).read_unaligned());
            }
            true
        }
        b'U' => {
            if buf.len() < 35 {
                return false;
            }
            // SAFETY: buf.len() >= 35, all pointer reads within bounds.
            unsafe {
                event.kind = EventKind::OrderReplace;
                event.stock_locate = u16::from_be((ptr.add(1) as *const u16).read_unaligned());
                event.timestamp_ns = u64::from_be((ptr.add(3) as *const u64).read_unaligned()) & 0x0000_FFFF_FFFF_FFFF;
                event.order_id = u64::from_be((ptr.add(11) as *const u64).read_unaligned());
                event.new_order_id = u64::from_be((ptr.add(19) as *const u64).read_unaligned());
                event.shares = u32::from_be((ptr.add(27) as *const u32).read_unaligned());
                event.price = u32::from_be((ptr.add(31) as *const u32).read_unaligned());
            }
            true
        }
        b'P' => {
            if buf.len() < 44 {
                return false;
            }
            // SAFETY: buf.len() >= 44, all pointer reads within bounds.
            unsafe {
                event.kind = EventKind::Trade;
                event.stock_locate = u16::from_be((ptr.add(1) as *const u16).read_unaligned());
                event.timestamp_ns = u64::from_be((ptr.add(3) as *const u64).read_unaligned()) & 0x0000_FFFF_FFFF_FFFF;
                event.order_id = u64::from_be((ptr.add(11) as *const u64).read_unaligned());
                event.side = if *ptr.add(19) == b'B' { Side::Buy } else { Side::Sell };
                event.shares = u32::from_be((ptr.add(20) as *const u32).read_unaligned());
                event.symbol = (ptr.add(24) as *const [u8; 8]).read_unaligned();
                event.price = u32::from_be((ptr.add(32) as *const u32).read_unaligned());
                event.match_number = u64::from_be((ptr.add(36) as *const u64).read_unaligned());
            }
            true
        }
        _ => false,
    }
}

#[inline(always)]
pub fn parse_itch_message_scalar(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() >= 36 {
        let ptr = buf.as_ptr();
        // SAFETY: buf.len() >= 36, all pointer reads within bounds.
        if unsafe { *ptr } == b'A' {
            unsafe {
                event.kind = EventKind::AddOrder;
                event.stock_locate = u16::from_be((ptr.add(1) as *const u16).read_unaligned());
                event.timestamp_ns = u64::from_be((ptr.add(3) as *const u64).read_unaligned()) & 0x0000_FFFF_FFFF_FFFF;
                event.order_id = u64::from_be((ptr.add(11) as *const u64).read_unaligned());
                event.side = if *ptr.add(19) == b'B' { Side::Buy } else { Side::Sell };
                event.shares = u32::from_be((ptr.add(20) as *const u32).read_unaligned());
                event.symbol = (ptr.add(24) as *const [u8; 8]).read_unaligned();
                event.price = u32::from_be((ptr.add(32) as *const u32).read_unaligned());
            }
            return true;
        }
    }
    parse_non_add(buf, event)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn parse_itch_add_order_avx2(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 36 {
        return false;
    }
    let ptr = buf.as_ptr();
    let raw = _mm256_loadu_si256(ptr as *const __m256i);
    let mut storage = [0u8; 32];
    _mm256_storeu_si256(storage.as_mut_ptr() as *mut __m256i, raw);
    let stock_locate = u16::from_be_bytes([storage[1], storage[2]]);
    let ts = parse_timestamp_48(&[storage[5], storage[6], storage[7], storage[8], storage[9], storage[10]]);
    let order_id = u64::from_be_bytes([storage[11], storage[12], storage[13], storage[14], storage[15], storage[16], storage[17], storage[18]]);
    let side = if storage[19] == b'B' { Side::Buy } else { Side::Sell };
    let shares = u32::from_be_bytes([storage[20], storage[21], storage[22], storage[23]]);
    let mut symbol = [0u8; 8];
    symbol.copy_from_slice(&storage[24..32]);
    let price = u32::from_be_bytes([buf[32], buf[33], buf[34], buf[35]]);
    event.kind = EventKind::AddOrder;
    event.stock_locate = stock_locate;
    event.timestamp_ns = ts;
    event.order_id = order_id;
    event.side = side;
    event.shares = shares;
    event.symbol = symbol;
    event.price = price;
    true
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f", enable = "avx512bw", enable = "avx512dq", enable = "avx512vl")]
pub unsafe fn parse_itch_add_order_avx512(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 36 {
        return false;
    }
    let ptr = buf.as_ptr();
    let raw = _mm512_loadu_si512(ptr as *const __m512i);
    let mut storage = [0u8; 64];
    _mm512_storeu_si512(storage.as_mut_ptr() as *mut __m512i, raw);
    let stock_locate = u16::from_be_bytes([storage[1], storage[2]]);
    let ts = parse_timestamp_48(&[storage[5], storage[6], storage[7], storage[8], storage[9], storage[10]]);
    let order_id = u64::from_be_bytes([storage[11], storage[12], storage[13], storage[14], storage[15], storage[16], storage[17], storage[18]]);
    let side = if storage[19] == b'B' { Side::Buy } else { Side::Sell };
    let shares = u32::from_be_bytes([storage[20], storage[21], storage[22], storage[23]]);
    let mut symbol = [0u8; 8];
    symbol.copy_from_slice(&storage[24..32]);
    let price = u32::from_be_bytes([storage[32], storage[33], storage[34], storage[35]]);
    event.kind = EventKind::AddOrder;
    event.stock_locate = stock_locate;
    event.timestamp_ns = ts;
    event.order_id = order_id;
    event.side = side;
    event.shares = shares;
    event.symbol = symbol;
    event.price = price;
    true
}

#[inline(always)]
pub fn parse_itch_message(buf: &[u8], event: &mut MarketEvent) -> bool {
    let t_begin = probes::stage_begin(StageId::ItchParse);
    let ok = parse_itch_message_scalar(buf, event);
    probes::stage_end(StageId::ItchParse, t_begin);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_itch_add_order() {
        let mut raw = vec![0u8; 64];
        raw[0] = b'A';
        raw[1..3].copy_from_slice(&1u16.to_be_bytes());
        raw[3..5].copy_from_slice(&0u16.to_be_bytes());
        raw[5..11].copy_from_slice(&[0, 0, 1, 2, 3, 4]);
        raw[11..19].copy_from_slice(&999999u64.to_be_bytes());
        raw[19] = b'B';
        raw[20..24].copy_from_slice(&500u32.to_be_bytes());
        raw[24..32].copy_from_slice(b"AAPL    ");
        raw[32..36].copy_from_slice(&1850000u32.to_be_bytes());
        let mut event = MarketEvent::default();
        let ok = parse_itch_message(&raw, &mut event);
        assert!(ok);
        assert_eq!(event.kind, EventKind::AddOrder);
        assert_eq!(event.order_id, 999999);
        assert_eq!(event.side, Side::Buy);
        assert_eq!(event.shares, 500);
        assert_eq!(&event.symbol, b"AAPL    ");
        assert_eq!(event.price, 1850000);
    }
}

