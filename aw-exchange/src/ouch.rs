pub const MSG_ENTER_ORDER: u8 = b'O';
pub const MSG_REPLACE_ORDER: u8 = b'U';
pub const MSG_CANCEL_ORDER: u8 = b'X';
pub const MSG_MODIFY_ORDER: u8 = b'M';
pub const MSG_SYSTEM_EVENT: u8 = b'S';
pub const MSG_ACCEPTED: u8 = b'A';
pub const MSG_REPLACED: u8 = b'U';
pub const MSG_CANCELED: u8 = b'C';
pub const MSG_EXECUTED: u8 = b'E';
pub const MSG_TRADE_BREAK: u8 = b'B';
pub const MSG_REJECTED: u8 = b'J';
pub const MSG_CANCEL_PENDING: u8 = b'P';
pub const MSG_CANCEL_REJECT: u8 = b'I';
pub const MSG_PRIORITY_UPDATE: u8 = b'T';
pub const MSG_MODIFIED: u8 = b'M';

pub const SIDE_BUY: u8 = b'B';
pub const SIDE_SELL: u8 = b'S';
pub const SIDE_SELL_SHORT: u8 = b'T';
pub const SIDE_SELL_SHORT_EXEMPT: u8 = b'E';

pub const TIF_DAY: u32 = 0;
pub const TIF_GTC: u32 = 0xFFFF;
pub const TIF_IOC: u32 = 3;

pub const DISPLAY_YES: u8 = b'Y';
pub const DISPLAY_NO: u8 = b'N';

pub const CAPACITY_AGENCY: u8 = b'A';
pub const CAPACITY_PRINCIPAL: u8 = b'P';

pub const SWEEP_YES: u8 = b'Y';
pub const SWEEP_NO: u8 = b'N';

pub const CROSS_NONE: u8 = b'N';

pub const CUSTOMER_RETAIL: u8 = b'R';
pub const CUSTOMER_PROFESSIONAL: u8 = b'P';

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OuchEnterOrder {
    pub order_token: [u8; 14],
    pub side: u8,
    pub shares: u32,
    pub stock: [u8; 8],
    pub price: u32,
    pub time_in_force: u32,
    pub firm: [u8; 4],
    pub display: u8,
    pub capacity: u8,
    pub sweep: u8,
    pub min_qty: u32,
    pub cross_type: u8,
    pub customer_type: u8,
    pub reserved: [u8; 4],
}

impl OuchEnterOrder {
    pub const WIRE_SIZE: usize = 38;

    #[inline]
    pub fn encode(&self, buf: &mut [u8]) -> usize {
        let total = 2 + 1 + Self::WIRE_SIZE;
        if buf.len() < total {
            return 0;
        }
        let len = (1 + Self::WIRE_SIZE) as u16;
        buf[0..2].copy_from_slice(&len.to_be_bytes());
        buf[2] = MSG_ENTER_ORDER;
        let p = &mut buf[3..];
        p[0..14].copy_from_slice(&self.order_token);
        p[14] = self.side;
        p[15..19].copy_from_slice(&self.shares.to_be_bytes());
        p[19..27].copy_from_slice(&self.stock);
        p[27..31].copy_from_slice(&self.price.to_be_bytes());
        p[31..35].copy_from_slice(&self.time_in_force.to_be_bytes());
        p[35..39].copy_from_slice(&self.firm);
        p[39] = self.display;
        p[40] = self.capacity;
        p[41] = self.sweep;
        p[42..46].copy_from_slice(&self.min_qty.to_be_bytes());
        p[46] = self.cross_type;
        p[47] = self.customer_type;
        p[48..52].copy_from_slice(&self.reserved);
        total
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OuchCancelOrder {
    pub order_token: [u8; 14],
    pub shares: u32,
}

impl OuchCancelOrder {
    pub const WIRE_SIZE: usize = 18;

    #[inline]
    pub fn encode(&self, buf: &mut [u8]) -> usize {
        let total = 2 + 1 + Self::WIRE_SIZE;
        if buf.len() < total {
            return 0;
        }
        let len = (1 + Self::WIRE_SIZE) as u16;
        buf[0..2].copy_from_slice(&len.to_be_bytes());
        buf[2] = MSG_CANCEL_ORDER;
        buf[3..17].copy_from_slice(&self.order_token);
        buf[17..21].copy_from_slice(&self.shares.to_be_bytes());
        total
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OuchReplaceOrder {
    pub existing_token: [u8; 14],
    pub replacement_token: [u8; 14],
    pub shares: u32,
    pub price: u32,
    pub time_in_force: u32,
    pub display: u8,
    pub reserved: [u8; 4],
}

impl OuchReplaceOrder {
    pub const WIRE_SIZE: usize = 34;

    #[inline]
    pub fn encode(&self, buf: &mut [u8]) -> usize {
        let total = 2 + 1 + Self::WIRE_SIZE;
        if buf.len() < total {
            return 0;
        }
        let len = (1 + Self::WIRE_SIZE) as u16;
        buf[0..2].copy_from_slice(&len.to_be_bytes());
        buf[2] = MSG_REPLACE_ORDER;
        let p = &mut buf[3..];
        p[0..14].copy_from_slice(&self.existing_token);
        p[14..28].copy_from_slice(&self.replacement_token);
        p[28..32].copy_from_slice(&self.shares.to_be_bytes());
        p[32..36].copy_from_slice(&self.price.to_be_bytes());
        p[36..40].copy_from_slice(&self.time_in_force.to_be_bytes());
        p[40] = self.display;
        p[41..45].copy_from_slice(&self.reserved);
        total
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuchInbound {
    SystemEvent { event_code: u8 },
    Accepted {
        order_token: [u8; 14],
        side: u8,
        shares: u32,
        stock: [u8; 8],
        price: u32,
        order_ref: u64,
    },
    Replaced {
        replacement_token: [u8; 14],
        side: u8,
        shares: u32,
        stock: [u8; 8],
        price: u32,
        order_ref: u64,
    },
    Canceled {
        order_token: [u8; 14],
        shares: u32,
        reason: u8,
    },
    Executed {
        order_token: [u8; 14],
        shares: u32,
        price: u32,
        liquidity: u8,
        match_number: u64,
    },
    Rejected {
        order_token: [u8; 14],
        reason: u8,
    },
    CancelPending {
        order_token: [u8; 14],
    },
    CancelReject {
        order_token: [u8; 14],
    },
    PriorityUpdate {
        order_token: [u8; 14],
        price: u32,
        shares: u32,
    },
    Modified {
        order_token: [u8; 14],
        shares: u32,
    },
    TradeBreak {
        match_number: u64,
    },
    Unknown { msg_type: u8, len: u16 },
}

#[inline]
fn read_u32_be(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[inline]
fn read_u64_be(buf: &[u8], off: usize) -> u64 {
    u64::from_be_bytes([
        buf[off], buf[off + 1], buf[off + 2], buf[off + 3],
        buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7],
    ])
}

#[inline]
fn copy_token(buf: &[u8], off: usize) -> [u8; 14] {
    let mut t = [0u8; 14];
    t.copy_from_slice(&buf[off..off + 14]);
    t
}

#[inline]
fn copy_stock(buf: &[u8], off: usize) -> [u8; 8] {
    let mut s = [0u8; 8];
    s.copy_from_slice(&buf[off..off + 8]);
    s
}

pub fn parse_ouch_inbound(buf: &[u8]) -> Option<(OuchInbound, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let msg_len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    let total = 2 + msg_len;
    if buf.len() < total || msg_len < 1 {
        return None;
    }

    let msg_type = buf[2];
    let payload = &buf[3..total];

    let result = match msg_type {
        MSG_SYSTEM_EVENT if payload.len() >= 5 => {
            OuchInbound::SystemEvent {
                event_code: payload[4],
            }
        }
        MSG_ACCEPTED if payload.len() >= 35 => {
            OuchInbound::Accepted {
                order_token: copy_token(payload, 0),
                side: payload[14],
                shares: read_u32_be(payload, 15),
                stock: copy_stock(payload, 19),
                price: read_u32_be(payload, 27),
                order_ref: read_u64_be(payload, 31),
            }
        }
        MSG_CANCELED if payload.len() >= 19 => {
            OuchInbound::Canceled {
                order_token: copy_token(payload, 0),
                shares: read_u32_be(payload, 14),
                reason: payload[18],
            }
        }
        MSG_EXECUTED if payload.len() >= 27 => {
            OuchInbound::Executed {
                order_token: copy_token(payload, 0),
                shares: read_u32_be(payload, 14),
                price: read_u32_be(payload, 18),
                liquidity: payload[22],
                match_number: read_u64_be(payload, 23),
            }
        }
        MSG_REJECTED if payload.len() >= 15 => {
            OuchInbound::Rejected {
                order_token: copy_token(payload, 0),
                reason: payload[14],
            }
        }
        MSG_CANCEL_PENDING if payload.len() >= 14 => {
            OuchInbound::CancelPending {
                order_token: copy_token(payload, 0),
            }
        }
        MSG_CANCEL_REJECT if payload.len() >= 14 => {
            OuchInbound::CancelReject {
                order_token: copy_token(payload, 0),
            }
        }
        MSG_PRIORITY_UPDATE if payload.len() >= 22 => {
            OuchInbound::PriorityUpdate {
                order_token: copy_token(payload, 0),
                price: read_u32_be(payload, 14),
                shares: read_u32_be(payload, 18),
            }
        }
        MSG_MODIFIED if payload.len() >= 18 => {
            OuchInbound::Modified {
                order_token: copy_token(payload, 0),
                shares: read_u32_be(payload, 14),
            }
        }
        MSG_TRADE_BREAK if payload.len() >= 8 => {
            OuchInbound::TradeBreak {
                match_number: read_u64_be(payload, 0),
            }
        }
        _ => OuchInbound::Unknown {
            msg_type,
            len: msg_len as u16,
        },
    };

    Some((result, total))
}

pub struct OuchFramer {
    buf: Vec<u8>,
    read_pos: usize,
    write_pos: usize,
}

impl OuchFramer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0u8; capacity],
            read_pos: 0,
            write_pos: 0,
        }
    }

    #[inline]
    pub fn write_buf(&mut self) -> &mut [u8] {
        &mut self.buf[self.write_pos..]
    }

    #[inline]
    pub fn advance_write(&mut self, n: usize) {
        self.write_pos += n;
    }

    #[inline]
    pub fn next_message(&mut self) -> Option<OuchInbound> {
        if self.write_pos - self.read_pos < 2 {
            return None;
        }
        match parse_ouch_inbound(&self.buf[self.read_pos..self.write_pos]) {
            Some((msg, consumed)) => {
                self.read_pos += consumed;
                if self.read_pos == self.write_pos {
                    self.read_pos = 0;
                    self.write_pos = 0;
                }
                Some(msg)
            }
            None => None,
        }
    }

    pub fn compact(&mut self) {
        if self.read_pos > 0 {
            let remaining = self.write_pos - self.read_pos;
            self.buf.copy_within(self.read_pos..self.write_pos, 0);
            self.read_pos = 0;
            self.write_pos = remaining;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_enter_order() {
        let mut buf = [0u8; 128];
        let order = OuchEnterOrder {
            order_token: *b"ORD00000000001",
            side: SIDE_BUY,
            shares: 100,
            stock: *b"AAPL    ",
            price: 1855000,
            time_in_force: TIF_DAY,
            firm: *b"TEST",
            display: DISPLAY_YES,
            capacity: CAPACITY_AGENCY,
            sweep: SWEEP_NO,
            min_qty: 0,
            cross_type: CROSS_NONE,
            customer_type: CUSTOMER_RETAIL,
            reserved: [0; 4],
        };
        let n = order.encode(&mut buf);
        assert_eq!(n, 2 + 1 + 38);
        let len = u16::from_be_bytes([buf[0], buf[1]]);
        assert_eq!(len, 39);
        assert_eq!(buf[2], MSG_ENTER_ORDER);
        assert_eq!(&buf[3..17], b"ORD00000000001");
        assert_eq!(buf[17], SIDE_BUY);
    }

    #[test]
    fn encode_cancel_order() {
        let mut buf = [0u8; 64];
        let cancel = OuchCancelOrder {
            order_token: *b"ORD00000000001",
            shares: 50,
        };
        let n = cancel.encode(&mut buf);
        assert_eq!(n, 2 + 1 + 18);
        assert_eq!(buf[2], MSG_CANCEL_ORDER);
    }

    #[test]
    fn parse_accepted() {
        let mut wire = vec![0u8; 2 + 1 + 55];
        let len: u16 = 56;
        wire[0..2].copy_from_slice(&len.to_be_bytes());
        wire[2] = MSG_ACCEPTED;
        wire[3..17].copy_from_slice(b"ORD00000000001");
        wire[17] = SIDE_BUY;
        wire[18..22].copy_from_slice(&100u32.to_be_bytes());
        wire[22..30].copy_from_slice(b"AAPL    ");
        wire[30..34].copy_from_slice(&1855000u32.to_be_bytes());
        wire[34..42].copy_from_slice(&42u64.to_be_bytes());

        let (msg, consumed) = parse_ouch_inbound(&wire).unwrap();
        assert_eq!(consumed, 2 + 56);
        match msg {
            OuchInbound::Accepted {
                order_token,
                side,
                shares,
                price,
                order_ref,
                ..
            } => {
                assert_eq!(&order_token, b"ORD00000000001");
                assert_eq!(side, SIDE_BUY);
                assert_eq!(shares, 100);
                assert_eq!(price, 1855000);
                assert_eq!(order_ref, 42);
            }
            _ => panic!("expected Accepted"),
        }
    }

    #[test]
    fn parse_executed() {
        let mut wire = vec![0u8; 2 + 1 + 36];
        let len: u16 = 37;
        wire[0..2].copy_from_slice(&len.to_be_bytes());
        wire[2] = MSG_EXECUTED;
        wire[3..17].copy_from_slice(b"ORD00000000001");
        wire[17..21].copy_from_slice(&100u32.to_be_bytes());
        wire[21..25].copy_from_slice(&1855000u32.to_be_bytes());
        wire[25] = b'A';
        wire[26..34].copy_from_slice(&999u64.to_be_bytes());

        let (msg, consumed) = parse_ouch_inbound(&wire).unwrap();
        assert_eq!(consumed, 2 + 37);
        match msg {
            OuchInbound::Executed {
                shares,
                price,
                match_number,
                ..
            } => {
                assert_eq!(shares, 100);
                assert_eq!(price, 1855000);
                assert_eq!(match_number, 999);
            }
            _ => panic!("expected Executed"),
        }
    }

    #[test]
    fn framer_multi_message() {
        let mut framer = OuchFramer::new(4096);

        let mut wire1 = vec![0u8; 2 + 1 + 14];
        let len1: u16 = 15;
        wire1[0..2].copy_from_slice(&len1.to_be_bytes());
        wire1[2] = MSG_CANCEL_PENDING;
        wire1[3..17].copy_from_slice(b"ORD00000000001");

        let mut wire2 = vec![0u8; 2 + 1 + 14];
        let len2: u16 = 15;
        wire2[0..2].copy_from_slice(&len2.to_be_bytes());
        wire2[2] = MSG_CANCEL_REJECT;
        wire2[3..17].copy_from_slice(b"ORD00000000002");

        let w = framer.write_buf();
        w[..wire1.len()].copy_from_slice(&wire1);
        w[wire1.len()..wire1.len() + wire2.len()].copy_from_slice(&wire2);
        framer.advance_write(wire1.len() + wire2.len());

        let m1 = framer.next_message().unwrap();
        assert!(matches!(m1, OuchInbound::CancelPending { .. }));

        let m2 = framer.next_message().unwrap();
        assert!(matches!(m2, OuchInbound::CancelReject { .. }));

        assert!(framer.next_message().is_none());
    }
}
