use wire_exchange::order::{EventKind, MarketEvent, Side};
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SbeHeader {
    pub block_length: u16,
    pub template_id: u16,
    pub schema_id: u16,
    pub version: u16,
}
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbeAddOrder {
    pub price: u32,
    pub qty: u32,
    pub security_id: u32,
    pub order_id: u64,
    pub side: u8,
    pub _pad: [u8; 3],
}
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbeOrderExecuted {
    pub price: u32,
    pub qty: u32,
    pub security_id: u32,
    pub order_id: u64,
    pub match_id: u64,
}
#[inline(always)]
pub fn parse_sbe_add_order_le(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 24 {
        return false;
    }
    // SAFETY: Transmuting Little-Endian SBE raw payload pointer.
    let sbe_add = unsafe { &*(buf.as_ptr() as *const SbeAddOrder) };
    event.kind = EventKind::AddOrder;
    event.order_id = u64::from_le(sbe_add.order_id);
    event.price = u32::from_le(sbe_add.price);
    event.shares = u32::from_le(sbe_add.qty);
    event.side = if sbe_add.side == 1 { Side::Buy } else { Side::Sell };
    let sec_id = u32::from_le(sbe_add.security_id);
    let sec_bytes = sec_id.to_le_bytes();
    event.symbol[0..4].copy_from_slice(&sec_bytes);
    event.symbol[4..8].copy_from_slice(b"    ");
    true
}
#[inline(always)]
pub fn parse_sbe_message(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 8 {
        return false;
    }
    // SAFETY: Transmuting Little-Endian SBE header.
    let hdr = unsafe { &*(buf.as_ptr() as *const SbeHeader) };
    let template_id = u16::from_le(hdr.template_id);
    match template_id {
        101 => parse_sbe_add_order_le(&buf[8..], event),
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_sbe_add_order() {
        let mut buf = vec![0u8; 32];
        let hdr = SbeHeader {
            block_length: 24u16.to_le(),
            template_id: 101u16.to_le(),
            schema_id: 1u16.to_le(),
            version: 1u16.to_le(),
        };
        // SAFETY: Direct copy of header struct into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, buf.as_mut_ptr(), 8);
        }
        let add = SbeAddOrder {
            price: 18500u32.to_le(),
            qty: 500u32.to_le(),
            security_id: 42u32.to_le(),
            order_id: 88888u64.to_le(),
            side: 1,
            _pad: [0; 3],
        };
        // SAFETY: Copying add order payload into buffer payload offset.
        unsafe {
            std::ptr::copy_nonoverlapping(&add as *const _ as *const u8, buf.as_mut_ptr().add(8), 24);
        }
        let mut event = MarketEvent::default();
        let ok = parse_sbe_message(&buf, &mut event);
        assert!(ok);
        assert_eq!(event.kind, EventKind::AddOrder);
        assert_eq!(event.order_id, 88888);
        assert_eq!(event.price, 18500);
        assert_eq!(event.shares, 500);
        assert_eq!(event.side, Side::Buy);
    }
}
