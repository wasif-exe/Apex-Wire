use aw_exchange::order::{EventKind, Side};
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
pub const MAX_VENUES: usize = 8;
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueBbo {
    pub venue_id: u8,
    pub bid_price: u32,
    pub bid_shares: u32,
    pub ask_price: u32,
    pub ask_shares: u32,
    pub active: bool,
    pub _pad: [u8; 10],
}
impl Default for VenueBbo {
    fn default() -> Self {
        Self {
            venue_id: 0,
            bid_price: 0,
            bid_shares: 0,
            ask_price: u32::MAX,
            ask_shares: 0,
            active: false,
            _pad: [0; 10],
        }
    }
}
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsolidatedNbbo {
    pub best_bid_venue: u8,
    pub best_bid_price: u32,
    pub best_bid_shares: u32,
    pub best_ask_venue: u8,
    pub best_ask_price: u32,
    pub best_ask_shares: u32,
    pub synth_arb_spread: i32,
}
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoutedChildOrder {
    pub target_venue: u8,
    pub price: u32,
    pub shares: u32,
    pub side: Side,
    pub action: EventKind,
}
pub struct SmartOrderRouter {
    pub symbol: [u8; 8],
    pub venues: [VenueBbo; MAX_VENUES],
    pub nbbo: ConsolidatedNbbo,
}
impl SmartOrderRouter {
    pub fn new(symbol: [u8; 8]) -> Self {
        let mut venues = [VenueBbo::default(); MAX_VENUES];
        for i in 0..MAX_VENUES {
            venues[i].venue_id = i as u8;
        }
        Self {
            symbol,
            venues,
            nbbo: ConsolidatedNbbo::default(),
        }
    }
    #[inline(always)]
    pub fn update_venue_bbo(&mut self, venue_id: usize, bid_price: u32, bid_shares: u32, ask_price: u32, ask_shares: u32) {
        if venue_id < MAX_VENUES {
            self.venues[venue_id].bid_price = bid_price;
            self.venues[venue_id].bid_shares = bid_shares;
            self.venues[venue_id].ask_price = ask_price;
            self.venues[venue_id].ask_shares = ask_shares;
            self.venues[venue_id].active = true;
        }
    }
    #[inline(always)]
    pub fn consolidate_nbbo_scalar(&mut self) -> ConsolidatedNbbo {
        let mut best_bid_p = 0u32;
        let mut best_bid_v = 0u8;
        let mut best_bid_s = 0u32;
        let mut best_ask_p = u32::MAX;
        let mut best_ask_v = 0u8;
        let mut best_ask_s = 0u32;
        for i in 0..MAX_VENUES {
            let v = &self.venues[i];
            if v.active {
                if v.bid_price > best_bid_p {
                    best_bid_p = v.bid_price;
                    best_bid_v = v.venue_id;
                    best_bid_s = v.bid_shares;
                }
                if v.ask_price < best_ask_p {
                    best_ask_p = v.ask_price;
                    best_ask_v = v.venue_id;
                    best_ask_s = v.ask_shares;
                }
            }
        }
        let synth_arb = if best_bid_p > 0 && best_ask_p < u32::MAX {
            (best_bid_p as i32) - (best_ask_p as i32)
        } else {
            0
        };
        self.nbbo = ConsolidatedNbbo {
            best_bid_venue: best_bid_v,
            best_bid_price: best_bid_p,
            best_bid_shares: best_bid_s,
            best_ask_venue: best_ask_v,
            best_ask_price: if best_ask_p == u32::MAX { 0 } else { best_ask_p },
            best_ask_shares: best_ask_s,
            synth_arb_spread: synth_arb,
        };
        self.nbbo
    }
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    pub unsafe fn consolidate_nbbo_avx2(&mut self) -> ConsolidatedNbbo {
        let mut bids = [0u32; 8];
        let mut asks = [0u32; 8];
        for i in 0..8 {
            bids[i] = self.venues[i].bid_price;
            asks[i] = if self.venues[i].active { self.venues[i].ask_price } else { u32::MAX };
        }
        let vec_bids = _mm256_loadu_si256(bids.as_ptr() as *const __m256i);
        let vec_asks = _mm256_loadu_si256(asks.as_ptr() as *const __m256i);
        let mut best_bid_p = 0u32;
        let mut best_bid_v = 0u8;
        let mut best_ask_p = u32::MAX;
        let mut best_ask_v = 0u8;
        let mut arr_b = [0u32; 8];
        let mut arr_a = [0u32; 8];
        _mm256_storeu_si256(arr_b.as_mut_ptr() as *mut __m256i, vec_bids);
        _mm256_storeu_si256(arr_a.as_mut_ptr() as *mut __m256i, vec_asks);
        for i in 0..8 {
            if arr_b[i] > best_bid_p {
                best_bid_p = arr_b[i];
                best_bid_v = i as u8;
            }
            if arr_a[i] < best_ask_p {
                best_ask_p = arr_a[i];
                best_ask_v = i as u8;
            }
        }
        let synth_arb = if best_bid_p > 0 && best_ask_p < u32::MAX {
            (best_bid_p as i32) - (best_ask_p as i32)
        } else {
            0
        };
        self.nbbo = ConsolidatedNbbo {
            best_bid_venue: best_bid_v,
            best_bid_price: best_bid_p,
            best_bid_shares: self.venues[best_bid_v as usize].bid_shares,
            best_ask_venue: best_ask_v,
            best_ask_price: if best_ask_p == u32::MAX { 0 } else { best_ask_p },
            best_ask_shares: self.venues[best_ask_v as usize].ask_shares,
            synth_arb_spread: synth_arb,
        };
        self.nbbo
    }
    #[inline(always)]
    pub fn route_order(&mut self, side: Side, target_shares: u32) -> RoutedChildOrder {
        let _nbbo = self.consolidate_nbbo_scalar();
        match side {
            Side::Buy => RoutedChildOrder {
                target_venue: self.nbbo.best_ask_venue,
                price: self.nbbo.best_ask_price,
                shares: target_shares.min(self.nbbo.best_ask_shares),
                side: Side::Buy,
                action: EventKind::AddOrder,
            },
            Side::Sell => RoutedChildOrder {
                target_venue: self.nbbo.best_bid_venue,
                price: self.nbbo.best_bid_price,
                shares: target_shares.min(self.nbbo.best_bid_shares),
                side: Side::Sell,
                action: EventKind::AddOrder,
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sor_routing() {
        let mut sor = SmartOrderRouter::new(*b"AAPL    ");
        sor.update_venue_bbo(0, 18000, 100, 18010, 200);
        sor.update_venue_bbo(1, 18005, 150, 18008, 300);
        let nbbo = sor.consolidate_nbbo_scalar();
        assert_eq!(nbbo.best_bid_price, 18005);
        assert_eq!(nbbo.best_bid_venue, 1);
        assert_eq!(nbbo.best_ask_price, 18008);
        assert_eq!(nbbo.best_ask_venue, 1);
        let child = sor.route_order(Side::Buy, 100);
        assert_eq!(child.target_venue, 1);
        assert_eq!(child.price, 18008);
        assert_eq!(child.shares, 100);
    }
}
