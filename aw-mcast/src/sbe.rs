use aw_exchange::order::{EventKind, MarketEvent, Side};

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
pub struct SbeOrderCancel {
    pub order_id: u64,
    pub security_id: u32,
    pub cancel_qty: u32,
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

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbeOrderDelete {
    pub order_id: u64,
    pub security_id: u32,
    pub _pad: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbeOrderReplace {
    pub original_order_id: u64,
    pub new_order_id: u64,
    pub price: u32,
    pub qty: u32,
    pub security_id: u32,
    pub side: u8,
    pub _pad: [u8; 3],
}

const TEMPLATE_ADD_ORDER: u16 = 101;
const TEMPLATE_ORDER_CANCEL: u16 = 102;
const TEMPLATE_ORDER_EXECUTED: u16 = 103;
const TEMPLATE_ORDER_DELETE: u16 = 104;
const TEMPLATE_ORDER_REPLACE: u16 = 105;

#[inline(always)]
fn decode_security_id(raw: u32, out: &mut [u8; 8]) {
    let sec_bytes = raw.to_le_bytes();
    out[0..4].copy_from_slice(&sec_bytes);
    out[4..8].copy_from_slice(b"    ");
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
    decode_security_id(u32::from_le(sbe_add.security_id), &mut event.symbol);
    true
}

#[inline(always)]
pub fn parse_sbe_order_cancel_le(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 16 {
        return false;
    }
    // SAFETY: Transmuting Little-Endian SBE cancel payload pointer.
    let sbe = unsafe { &*(buf.as_ptr() as *const SbeOrderCancel) };
    event.kind = EventKind::OrderCancel;
    event.order_id = u64::from_le(sbe.order_id);
    event.shares = u32::from_le(sbe.cancel_qty);
    decode_security_id(u32::from_le(sbe.security_id), &mut event.symbol);
    true
}

#[inline(always)]
pub fn parse_sbe_order_executed_le(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 28 {
        return false;
    }
    // SAFETY: Transmuting Little-Endian SBE execute payload pointer.
    let sbe = unsafe { &*(buf.as_ptr() as *const SbeOrderExecuted) };
    event.kind = EventKind::OrderExecuted;
    event.order_id = u64::from_le(sbe.order_id);
    event.price = u32::from_le(sbe.price);
    event.shares = u32::from_le(sbe.qty);
    event.match_number = u64::from_le(sbe.match_id);
    decode_security_id(u32::from_le(sbe.security_id), &mut event.symbol);
    true
}

#[inline(always)]
pub fn parse_sbe_order_delete_le(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 16 {
        return false;
    }
    // SAFETY: Transmuting Little-Endian SBE delete payload pointer.
    let sbe = unsafe { &*(buf.as_ptr() as *const SbeOrderDelete) };
    event.kind = EventKind::OrderDelete;
    event.order_id = u64::from_le(sbe.order_id);
    decode_security_id(u32::from_le(sbe.security_id), &mut event.symbol);
    true
}

#[inline(always)]
pub fn parse_sbe_order_replace_le(buf: &[u8], event: &mut MarketEvent) -> bool {
    if buf.len() < 32 {
        return false;
    }
    // SAFETY: Transmuting Little-Endian SBE replace payload pointer.
    let sbe = unsafe { &*(buf.as_ptr() as *const SbeOrderReplace) };
    event.kind = EventKind::OrderReplace;
    event.order_id = u64::from_le(sbe.original_order_id);
    event.new_order_id = u64::from_le(sbe.new_order_id);
    event.price = u32::from_le(sbe.price);
    event.shares = u32::from_le(sbe.qty);
    event.side = if sbe.side == 1 { Side::Buy } else { Side::Sell };
    decode_security_id(u32::from_le(sbe.security_id), &mut event.symbol);
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
        TEMPLATE_ADD_ORDER => parse_sbe_add_order_le(&buf[8..], event),
        TEMPLATE_ORDER_CANCEL => parse_sbe_order_cancel_le(&buf[8..], event),
        TEMPLATE_ORDER_EXECUTED => parse_sbe_order_executed_le(&buf[8..], event),
        TEMPLATE_ORDER_DELETE => parse_sbe_order_delete_le(&buf[8..], event),
        TEMPLATE_ORDER_REPLACE => parse_sbe_order_replace_le(&buf[8..], event),
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

    #[test]
    fn parse_sbe_order_cancel() {
        let mut buf = vec![0u8; 32];
        let hdr = SbeHeader {
            block_length: 16u16.to_le(),
            template_id: 102u16.to_le(),
            schema_id: 1u16.to_le(),
            version: 1u16.to_le(),
        };
        // SAFETY: Copying header into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, buf.as_mut_ptr(), 8);
        }
        let cancel = SbeOrderCancel {
            order_id: 88888u64.to_le(),
            security_id: 42u32.to_le(),
            cancel_qty: 200u32.to_le(),
        };
        // SAFETY: Copying cancel payload into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&cancel as *const _ as *const u8, buf.as_mut_ptr().add(8), 16);
        }
        let mut event = MarketEvent::default();
        let ok = parse_sbe_message(&buf, &mut event);
        assert!(ok);
        assert_eq!(event.kind, EventKind::OrderCancel);
        assert_eq!(event.order_id, 88888);
        assert_eq!(event.shares, 200);
    }

    #[test]
    fn parse_sbe_order_executed() {
        let mut buf = vec![0u8; 40];
        let hdr = SbeHeader {
            block_length: 28u16.to_le(),
            template_id: 103u16.to_le(),
            schema_id: 1u16.to_le(),
            version: 1u16.to_le(),
        };
        // SAFETY: Copying header into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, buf.as_mut_ptr(), 8);
        }
        let exec = SbeOrderExecuted {
            price: 18500u32.to_le(),
            qty: 100u32.to_le(),
            security_id: 42u32.to_le(),
            order_id: 88888u64.to_le(),
            match_id: 999u64.to_le(),
        };
        // SAFETY: Copying execute payload into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&exec as *const _ as *const u8, buf.as_mut_ptr().add(8), 28);
        }
        let mut event = MarketEvent::default();
        let ok = parse_sbe_message(&buf, &mut event);
        assert!(ok);
        assert_eq!(event.kind, EventKind::OrderExecuted);
        assert_eq!(event.order_id, 88888);
        assert_eq!(event.match_number, 999);
        assert_eq!(event.shares, 100);
    }

    #[test]
    fn parse_sbe_order_delete() {
        let mut buf = vec![0u8; 32];
        let hdr = SbeHeader {
            block_length: 16u16.to_le(),
            template_id: 104u16.to_le(),
            schema_id: 1u16.to_le(),
            version: 1u16.to_le(),
        };
        // SAFETY: Copying header into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, buf.as_mut_ptr(), 8);
        }
        let del = SbeOrderDelete {
            order_id: 77777u64.to_le(),
            security_id: 42u32.to_le(),
            _pad: 0,
        };
        // SAFETY: Copying delete payload into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&del as *const _ as *const u8, buf.as_mut_ptr().add(8), 16);
        }
        let mut event = MarketEvent::default();
        let ok = parse_sbe_message(&buf, &mut event);
        assert!(ok);
        assert_eq!(event.kind, EventKind::OrderDelete);
        assert_eq!(event.order_id, 77777);
    }

    #[test]
    fn parse_sbe_order_replace() {
        let mut buf = vec![0u8; 48];
        let hdr = SbeHeader {
            block_length: 32u16.to_le(),
            template_id: 105u16.to_le(),
            schema_id: 1u16.to_le(),
            version: 1u16.to_le(),
        };
        // SAFETY: Copying header into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&hdr as *const _ as *const u8, buf.as_mut_ptr(), 8);
        }
        let repl = SbeOrderReplace {
            original_order_id: 88888u64.to_le(),
            new_order_id: 99999u64.to_le(),
            price: 18600u32.to_le(),
            qty: 300u32.to_le(),
            security_id: 42u32.to_le(),
            side: 2,
            _pad: [0; 3],
        };
        // SAFETY: Copying replace payload into buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(&repl as *const _ as *const u8, buf.as_mut_ptr().add(8), 32);
        }
        let mut event = MarketEvent::default();
        let ok = parse_sbe_message(&buf, &mut event);
        assert!(ok);
        assert_eq!(event.kind, EventKind::OrderReplace);
        assert_eq!(event.order_id, 88888);
        assert_eq!(event.new_order_id, 99999);
        assert_eq!(event.price, 18600);
        assert_eq!(event.shares, 300);
        assert_eq!(event.side, Side::Sell);
    }
}
