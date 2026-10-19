#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    #[default]
    Buy = 1,
    Sell = 2,
}
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventKind {
    #[default]
    None = 0,
    AddOrder = 1,
    OrderExecuted = 2,
    OrderCancel = 3,
    OrderDelete = 4,
    OrderReplace = 5,
    Trade = 6,
    BboUpdate = 7,
}
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketEvent {
    pub timestamp_ns: u64,
    pub order_id: u64,
    pub new_order_id: u64,
    pub price: u32,
    pub shares: u32,
    pub match_number: u64,
    pub symbol: [u8; 8],
    pub kind: EventKind,
    pub side: Side,
    pub stock_locate: u16,
    pub _pad: [u8; 12],
}
impl Default for MarketEvent {
    #[inline(always)]
    fn default() -> Self {
        Self {
            timestamp_ns: 0,
            order_id: 0,
            new_order_id: 0,
            price: 0,
            shares: 0,
            match_number: 0,
            symbol: [0; 8],
            kind: EventKind::None,
            side: Side::Buy,
            stock_locate: 0,
            _pad: [0; 12],
        }
    }
}

