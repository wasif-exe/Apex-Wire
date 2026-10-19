use crate::order::{EventKind, MarketEvent, Side};
use aw_core::probes::{self, StageId};
pub const MAX_ORDERS_INDEXED: usize = 65536;
pub const PRICE_SLOTS: usize = 65536;
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
        }
    }
    #[inline(always)]
    fn slot_idx(price: u32) -> usize {
        (price as usize) & (PRICE_SLOTS - 1)
    }
    #[inline(always)]
    pub fn bbo(&self) -> Bbo {
        let spread = if self.best_ask_price > self.best_bid_price && self.best_bid_price > 0 && self.best_ask_price < u32::MAX {
            self.best_ask_price - self.best_bid_price
        } else {
            0
        };
        Bbo {
            bid_price: self.best_bid_price,
            bid_shares: self.best_bid_shares,
            ask_price: if self.best_ask_price == u32::MAX { 0 } else { self.best_ask_price },
            ask_shares: self.best_ask_shares,
            spread,
        }
    }
    #[inline(always)]
    pub fn apply_event(&mut self, event: &MarketEvent) -> Option<Bbo> {
        let t_begin = probes::stage_begin(StageId::LobUpdate);
        match event.kind {
            EventKind::AddOrder => {
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
                        if event.price <= self.best_ask_price {
                            self.best_ask_price = event.price;
                            self.best_ask_shares = lvl.shares;
                        }
                    }
                }
            }
            EventKind::OrderExecuted => {
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
                            if lvl.order_count == 0 { lvl.active = false; }
                            if price == self.best_bid_price {
                                self.best_bid_shares = lvl.shares;
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(event.shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 { lvl.active = false; }
                            if price == self.best_ask_price {
                                self.best_ask_shares = lvl.shares;
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
                    let shares = if event.kind == EventKind::OrderCancel { event.shares } else { slot.shares };
                    slot.active = false;
                    let p_idx = Self::slot_idx(price);
                    match side {
                        Side::Buy => {
                            let lvl = &mut self.bid_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 { lvl.active = false; }
                            if price == self.best_bid_price {
                                self.best_bid_shares = lvl.shares;
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[p_idx];
                            lvl.shares = lvl.shares.saturating_sub(shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 { lvl.active = false; }
                            if price == self.best_ask_price {
                                self.best_ask_shares = lvl.shares;
                            }
                        }
                    }
                }
            }
            EventKind::OrderReplace => {
                let orig_slot = &mut self.order_map[(event.order_id as usize) & self.order_map_mask];
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
                            if lvl.order_count == 0 { lvl.active = false; }
                            if old_price == self.best_bid_price {
                                self.best_bid_shares = lvl.shares;
                            }
                        }
                        Side::Sell => {
                            let lvl = &mut self.ask_levels[old_p_idx];
                            lvl.shares = lvl.shares.saturating_sub(old_shares as u64);
                            lvl.order_count = lvl.order_count.saturating_sub(1);
                            if lvl.order_count == 0 { lvl.active = false; }
                            if old_price == self.best_ask_price {
                                self.best_ask_shares = lvl.shares;
                            }
                        }
                    }
                    let new_slot = &mut self.order_map[(event.new_order_id as usize) & self.order_map_mask];
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
}

