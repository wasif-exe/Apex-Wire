use crate::order::{EventKind, MarketEvent, Side};
use aw_core::probes::{self, StageId};

pub const MAX_ORDERS_INDEXED: usize = 65536;
pub const PRICE_SLOTS: usize = 65536;
const BITMAP_WORDS: usize = PRICE_SLOTS / 64;

#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PriceLevel {
    pub price: u32,
    pub shares: u64,
    pub order_count: u32,
    pub _pad: [u8; 12],
}

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bbo {
    pub bid_price: u32,
    pub bid_shares: u64,
    pub ask_price: u32,
    pub ask_shares: u64,
    pub spread: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct OrderEntry {
    price: u32,
    shares: u32,
    side: Side,
    active: bool,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct FlatLevel {
    pub shares: u64,
    pub order_count: u32,
    pub active: bool,
    pub _pad: [u8; 3],
}

pub struct LimitOrderBook {
    pub symbol: [u8; 8],
    pub best_bid_price: u32,
    pub best_bid_shares: u64,
    pub best_ask_price: u32,
    pub best_ask_shares: u64,
    order_map: Vec<OrderEntry>,
    order_map_mask: usize,
    bid_levels: Vec<FlatLevel>,
    ask_levels: Vec<FlatLevel>,
    bid_bitmap: Vec<u64>,
    ask_bitmap: Vec<u64>,
}

impl LimitOrderBook {
    pub fn new(symbol: [u8; 8]) -> Self {
        let cap = MAX_ORDERS_INDEXED;
        Self {
            symbol,
            best_bid_price: 0,
            best_bid_shares: 0,
            best_ask_price: u32::MAX,
            best_ask_shares: 0,
            order_map: vec![OrderEntry::default(); cap],
            order_map_mask: cap - 1,
            bid_levels: vec![FlatLevel::default(); PRICE_SLOTS],
            ask_levels: vec![FlatLevel::default(); PRICE_SLOTS],
            bid_bitmap: vec![0u64; BITMAP_WORDS],
            ask_bitmap: vec![0u64; BITMAP_WORDS],
        }
    }

    #[inline(always)]
    fn slot_idx(price: u32) -> usize {
        (price as usize) & (PRICE_SLOTS - 1)
    }

    #[inline(always)]
    fn bitmap_set(bitmap: &mut [u64], price: u32) {
        let idx = (price as usize) & (PRICE_SLOTS - 1);
        bitmap[idx >> 6] |= 1u64 << (idx & 63);
    }

    #[inline(always)]
    fn bitmap_clear(bitmap: &mut [u64], price: u32) {
        let idx = (price as usize) & (PRICE_SLOTS - 1);
        bitmap[idx >> 6] &= !(1u64 << (idx & 63));
    }

    #[inline(always)]
    fn find_next_best_bid(bitmap: &[u64], below_price: u32) -> u32 {
        if below_price == 0 {
            return 0;
        }
        let start_idx = ((below_price - 1) as usize) & (PRICE_SLOTS - 1);
        let word_idx = start_idx >> 6;
        let bit_pos = start_idx & 63;
        let mask = (1u64 << (bit_pos + 1)).wrapping_sub(1);
        let w = bitmap[word_idx] & mask;
        if w != 0 {
            let highest = 63 - w.leading_zeros();
            return (word_idx as u32) * 64 + highest;
        }
        let mut wi = word_idx;
        while wi >= 4 {
            let base = wi - 4;
            // SAFETY: base + 4 <= BITMAP_WORDS, pointer within Vec allocation.
            let v = unsafe {
                core::arch::x86_64::_mm256_loadu_si256(
                    bitmap.as_ptr().add(base) as *const core::arch::x86_64::__m256i
                )
            };
            if unsafe { core::arch::x86_64::_mm256_testz_si256(v, v) } == 0 {
                for j in (0..4).rev() {
                    let idx = base + j;
                    if bitmap[idx] != 0 {
                        return (idx as u32) * 64 + (63 - bitmap[idx].leading_zeros());
                    }
                }
            }
            wi = base;
        }
        while wi > 0 {
            wi -= 1;
            if bitmap[wi] != 0 {
                return (wi as u32) * 64 + (63 - bitmap[wi].leading_zeros());
            }
        }
        0
    }

    #[inline(always)]
    fn find_next_best_ask(bitmap: &[u64], above_price: u32) -> u32 {
        if above_price >= PRICE_SLOTS as u32 - 1 {
            return u32::MAX;
        }
        let start_idx = ((above_price + 1) as usize) & (PRICE_SLOTS - 1);
        let word_idx = start_idx >> 6;
        let bit_pos = start_idx & 63;
        let mask = !((1u64 << bit_pos).wrapping_sub(1));
        let w = bitmap[word_idx] & mask;
        if w != 0 {
            let lowest = w.trailing_zeros();
            return (word_idx as u32) * 64 + lowest;
        }
        let mut wi = word_idx + 1;
        while wi + 4 <= BITMAP_WORDS {
            // SAFETY: wi + 4 <= BITMAP_WORDS, pointer within Vec allocation.
            let v = unsafe {
                core::arch::x86_64::_mm256_loadu_si256(
                    bitmap.as_ptr().add(wi) as *const core::arch::x86_64::__m256i
                )
            };
            if unsafe { core::arch::x86_64::_mm256_testz_si256(v, v) } == 0 {
                for j in 0..4 {
                    if bitmap[wi + j] != 0 {
                        return ((wi + j) as u32) * 64 + bitmap[wi + j].trailing_zeros();
                    }
                }
            }
            wi += 4;
        }
        while wi < BITMAP_WORDS {
            if bitmap[wi] != 0 {
                return (wi as u32) * 64 + bitmap[wi].trailing_zeros();
            }
            wi += 1;
        }
        u32::MAX
    }

    #[inline(always)]
    pub fn bbo(&self) -> Bbo {
        let spread = if self.best_ask_price > self.best_bid_price
            && self.best_bid_price > 0
            && self.best_ask_price < u32::MAX
        {
            self.best_ask_price - self.best_bid_price
        } else {
            0
        };
        Bbo {
            bid_price: self.best_bid_price,
            bid_shares: self.best_bid_shares,
            ask_price: if self.best_ask_price == u32::MAX {
                0
            } else {
                self.best_ask_price
            },
            ask_shares: self.best_ask_shares,
            spread,
        }
    }

    #[inline(always)]
    pub fn apply_event(&mut self, event: &MarketEvent) -> Option<Bbo> {
        let t_begin = probes::stage_begin(StageId::LobUpdate);
        match event.kind {
            EventKind::AddOrder | EventKind::AddOrderMpid => {
                let slot = &mut self.order_map[(event.order_id as usize) & self.order_map_mask];
                slot.price = event.price;
                slot.shares = event.shares;
                slot.side = event.side;
                slot.active = true;
                let p_idx = Self::slot_idx(event.price);
                match event.side {
                    Side::Buy => {
                        let lvl = &mut self.bid_levels[p_idx];
                        lvl.shares = lvl.shares.saturating_add(event.shares as u64);
                        lvl.order_count += 1;
                        lvl.active = true;
                        Self::bitmap_set(&mut self.bid_bitmap, event.price);
                        if event.price >= self.best_bid_price {
                            self.best_bid_price = event.price;
                            self.best_bid_shares = lvl.shares;
                        }
                    }
                    Side::Sell => {
                        let lvl = &mut self.ask_levels[p_idx];
                        lvl.shares = lvl.shares.saturating_add(event.shares as u64);
                        lvl.order_count += 1;
                        lvl.active = true;
                        Self::bitmap_set(&mut self.ask_bitmap, event.price);
                        if event.price <= self.best_ask_price {
                            self.best_ask_price = event.price;
                            self.best_ask_shares = lvl.shares;
                        }
                    }
                }
            }
            EventKind::OrderExecuted | EventKind::OrderExecutedWithPrice => {
                let slot = &mut self.order_map[(event.order_id as usize) & self.order_map_mask];
                if slot.active {
                    let price = slot.price;
                    let side = slot.side;
                    if slot.shares <= event.shares {
                        slot.active = false;
                    } else {
                        slot.shares -= event.shares;
                    }
                    let p_idx = Self::slot_idx(price);
                    match side {
                        Side::Buy => {
                            let lvl = &mut self.bid_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(event.shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 {
                                lvl.active = false;
                                Self::bitmap_clear(&mut self.bid_bitmap, price);
                            }
                            if price == self.best_bid_price {
                                if lvl.order_count == 0 {
                                    self.best_bid_price =
                                        Self::find_next_best_bid(&self.bid_bitmap, price);
                                    if self.best_bid_price > 0 {
                                        let ni = Self::slot_idx(self.best_bid_price);
                                        self.best_bid_shares = self.bid_levels[ni].shares;
                                    } else {
                                        self.best_bid_shares = 0;
                                    }
                                } else {
                                    self.best_bid_shares = lvl.shares;
                                }
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(event.shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 {
                                lvl.active = false;
                                Self::bitmap_clear(&mut self.ask_bitmap, price);
                            }
                            if price == self.best_ask_price {
                                if lvl.order_count == 0 {
                                    self.best_ask_price =
                                        Self::find_next_best_ask(&self.ask_bitmap, price);
                                    if self.best_ask_price < u32::MAX {
                                        let ni = Self::slot_idx(self.best_ask_price);
                                        self.best_ask_shares = self.ask_levels[ni].shares;
                                    } else {
                                        self.best_ask_shares = 0;
                                    }
                                } else {
                                    self.best_ask_shares = lvl.shares;
                                }
                            }
                        }
                    }
                }
            }
            EventKind::OrderCancel | EventKind::OrderDelete => {
                let slot = &mut self.order_map[(event.order_id as usize) & self.order_map_mask];
                if slot.active {
                    let price = slot.price;
                    let side = slot.side;
                    let shares = if event.kind == EventKind::OrderCancel {
                        event.shares
                    } else {
                        slot.shares
                    };
                    slot.active = false;
                    let p_idx = Self::slot_idx(price);
                    match side {
                        Side::Buy => {
                            let lvl = &mut self.bid_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 {
                                lvl.active = false;
                                Self::bitmap_clear(&mut self.bid_bitmap, price);
                            }
                            if price == self.best_bid_price {
                                if lvl.order_count == 0 {
                                    self.best_bid_price =
                                        Self::find_next_best_bid(&self.bid_bitmap, price);
                                    if self.best_bid_price > 0 {
                                        let ni = Self::slot_idx(self.best_bid_price);
                                        self.best_bid_shares = self.bid_levels[ni].shares;
                                    } else {
                                        self.best_bid_shares = 0;
                                    }
                                } else {
                                    self.best_bid_shares = lvl.shares;
                                }
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 {
                                lvl.active = false;
                                Self::bitmap_clear(&mut self.ask_bitmap, price);
                            }
                            if price == self.best_ask_price {
                                if lvl.order_count == 0 {
                                    self.best_ask_price =
                                        Self::find_next_best_ask(&self.ask_bitmap, price);
                                    if self.best_ask_price < u32::MAX {
                                        let ni = Self::slot_idx(self.best_ask_price);
                                        self.best_ask_shares = self.ask_levels[ni].shares;
                                    } else {
                                        self.best_ask_shares = 0;
                                    }
                                } else {
                                    self.best_ask_shares = lvl.shares;
                                }
                            }
                        }
                    }
                }
            }
            EventKind::OrderReplace => {
                let orig_slot =
                    &mut self.order_map[(event.order_id as usize) & self.order_map_mask];
                if orig_slot.active {
                    let old_price = orig_slot.price;
                    let old_shares = orig_slot.shares;
                    let side = orig_slot.side;
                    orig_slot.active = false;
                    let old_p_idx = Self::slot_idx(old_price);
                    match side {
                        Side::Buy => {
                            let lvl = &mut self.bid_levels[old_p_idx];
                            lvl.shares = lvl.shares.saturating_sub(old_shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 {
                                lvl.active = false;
                                Self::bitmap_clear(&mut self.bid_bitmap, old_price);
                            }
                            if old_price == self.best_bid_price {
                                if lvl.order_count == 0 {
                                    self.best_bid_price =
                                        Self::find_next_best_bid(&self.bid_bitmap, old_price);
                                    if self.best_bid_price > 0 {
                                        let ni = Self::slot_idx(self.best_bid_price);
                                        self.best_bid_shares = self.bid_levels[ni].shares;
                                    } else {
                                        self.best_bid_shares = 0;
                                    }
                                } else {
                                    self.best_bid_shares = lvl.shares;
                                }
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[old_p_idx];
                            lvl.shares = lvl.shares.saturating_sub(old_shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 {
                                lvl.active = false;
                                Self::bitmap_clear(&mut self.ask_bitmap, old_price);
                            }
                            if old_price == self.best_ask_price {
                                if lvl.order_count == 0 {
                                    self.best_ask_price =
                                        Self::find_next_best_ask(&self.ask_bitmap, old_price);
                                    if self.best_ask_price < u32::MAX {
                                        let ni = Self::slot_idx(self.best_ask_price);
                                        self.best_ask_shares = self.ask_levels[ni].shares;
                                    } else {
                                        self.best_ask_shares = 0;
                                    }
                                } else {
                                    self.best_ask_shares = lvl.shares;
                                }
                            }
                        }
                    }
                    let new_slot =
                        &mut self.order_map[(event.new_order_id as usize) & self.order_map_mask];
                    new_slot.price = event.price;
                    new_slot.shares = event.shares;
                    new_slot.side = side;
                    new_slot.active = true;
                    let new_p_idx = Self::slot_idx(event.price);
                    match side {
                        Side::Buy => {
                            let lvl = &mut self.bid_levels[new_p_idx];
                            lvl.shares = lvl.shares.saturating_add(event.shares as u64);
                            lvl.order_count += 1;
                            lvl.active = true;
                            Self::bitmap_set(&mut self.bid_bitmap, event.price);
                            if event.price >= self.best_bid_price {
                                self.best_bid_price = event.price;
                                self.best_bid_shares = lvl.shares;
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[new_p_idx];
                            lvl.shares = lvl.shares.saturating_add(event.shares as u64);
                            lvl.order_count += 1;
                            lvl.active = true;
                            Self::bitmap_set(&mut self.ask_bitmap, event.price);
                            if event.price <= self.best_ask_price {
                                self.best_ask_price = event.price;
                                self.best_ask_shares = lvl.shares;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        let bbo = self.bbo();
        probes::stage_end(StageId::LobUpdate, t_begin);
        Some(bbo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lob_bbo_tracking() {
        let mut lob = LimitOrderBook::new(*b"AAPL    ");
        let mut add_bid = MarketEvent::default();
        add_bid.kind = EventKind::AddOrder;
        add_bid.order_id = 1;
        add_bid.price = 18000;
        add_bid.shares = 100;
        add_bid.side = Side::Buy;
        lob.apply_event(&add_bid);
        let mut add_ask = MarketEvent::default();
        add_ask.kind = EventKind::AddOrder;
        add_ask.order_id = 2;
        add_ask.price = 18010;
        add_ask.shares = 200;
        add_ask.side = Side::Sell;
        lob.apply_event(&add_ask);
        let bbo = lob.bbo();
        assert_eq!(bbo.bid_price, 18000);
        assert_eq!(bbo.bid_shares, 100);
        assert_eq!(bbo.ask_price, 18010);
        assert_eq!(bbo.ask_shares, 200);
        assert_eq!(bbo.spread, 10);
    }

    #[test]
    fn lob_bid_depletion_scans_down() {
        let mut lob = LimitOrderBook::new(*b"AAPL    ");
        let mut e1 = MarketEvent::default();
        e1.kind = EventKind::AddOrder;
        e1.order_id = 1;
        e1.price = 18000;
        e1.shares = 100;
        e1.side = Side::Buy;
        lob.apply_event(&e1);
        let mut e2 = MarketEvent::default();
        e2.kind = EventKind::AddOrder;
        e2.order_id = 2;
        e2.price = 17990;
        e2.shares = 200;
        e2.side = Side::Buy;
        lob.apply_event(&e2);
        assert_eq!(lob.best_bid_price, 18000);
        let mut del = MarketEvent::default();
        del.kind = EventKind::OrderDelete;
        del.order_id = 1;
        lob.apply_event(&del);
        assert_eq!(lob.best_bid_price, 17990);
        assert_eq!(lob.best_bid_shares, 200);
    }

    #[test]
    fn lob_ask_depletion_scans_up() {
        let mut lob = LimitOrderBook::new(*b"AAPL    ");
        let mut e1 = MarketEvent::default();
        e1.kind = EventKind::AddOrder;
        e1.order_id = 10;
        e1.price = 18010;
        e1.shares = 100;
        e1.side = Side::Sell;
        lob.apply_event(&e1);
        let mut e2 = MarketEvent::default();
        e2.kind = EventKind::AddOrder;
        e2.order_id = 11;
        e2.price = 18020;
        e2.shares = 300;
        e2.side = Side::Sell;
        lob.apply_event(&e2);
        assert_eq!(lob.best_ask_price, 18010);
        let mut del = MarketEvent::default();
        del.kind = EventKind::OrderDelete;
        del.order_id = 10;
        lob.apply_event(&del);
        assert_eq!(lob.best_ask_price, 18020);
        assert_eq!(lob.best_ask_shares, 300);
    }

    #[test]
    fn lob_full_depletion_zeroes_bbo() {
        let mut lob = LimitOrderBook::new(*b"AAPL    ");
        let mut e1 = MarketEvent::default();
        e1.kind = EventKind::AddOrder;
        e1.order_id = 1;
        e1.price = 18000;
        e1.shares = 100;
        e1.side = Side::Buy;
        lob.apply_event(&e1);
        let mut del = MarketEvent::default();
        del.kind = EventKind::OrderDelete;
        del.order_id = 1;
        lob.apply_event(&del);
        assert_eq!(lob.best_bid_price, 0);
        assert_eq!(lob.best_bid_shares, 0);
    }
}


impl LimitOrderBook {
    #[inline(always)]
    pub fn shares_at_price(&self, price: u32, side: Side) -> u64 {
        let p_idx = Self::slot_idx(price);
        match side {
            Side::Buy => self.bid_levels[p_idx].shares,
            Side::Sell => self.ask_levels[p_idx].shares,
        }
    }

    #[inline(always)]
    pub fn order_info(&self, order_id: u64) -> Option<(u32, u32, Side)> {
        let slot = &self.order_map[(order_id as usize) & self.order_map_mask];
        if slot.active {
            Some((slot.price, slot.shares, slot.side))
        } else {
            None
        }
    }
}
