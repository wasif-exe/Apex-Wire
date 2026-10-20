use aw_exchange::lob::LimitOrderBook;
use aw_exchange::order::{EventKind, MarketEvent, Side};

pub struct PaperConfig {
    pub transit_latency_ns: u64,
    pub symbol: [u8; 8],
}

#[derive(Clone, Copy)]
pub struct PaperFill {
    pub timestamp_ns: u64,
    pub order_id: u64,
    pub price: u32,
    pub shares: u32,
    pub side: Side,
}

#[allow(dead_code)]
struct PaperOrder {
    order_id: u64,
    price: u32,
    shares: u32,
    remaining: u32,
    side: Side,
    live_time_ns: u64,
    ahead_volume: u64,
    active: bool,
}

pub struct PaperEngine {
    config: PaperConfig,
    lob: LimitOrderBook,
    orders: Vec<PaperOrder>,
    fills: Vec<PaperFill>,
    next_order_id: u64,
    current_time_ns: u64,
}

impl PaperEngine {
    pub fn new(config: PaperConfig) -> Self {
        Self {
            lob: LimitOrderBook::new(config.symbol),
            orders: Vec::new(),
            fills: Vec::new(),
            next_order_id: 1,
            current_time_ns: 0,
            config,
        }
    }

    pub fn submit_order(
        &mut self,
        price: u32,
        shares: u32,
        side: Side,
        timestamp_ns: u64,
    ) -> u64 {
        let oid = self.next_order_id;
        self.next_order_id += 1;
        let ahead = self.lob.shares_at_price(price, side);
        self.orders.push(PaperOrder {
            order_id: oid,
            price,
            shares,
            remaining: shares,
            side,
            live_time_ns: timestamp_ns + self.config.transit_latency_ns,
            ahead_volume: ahead,
            active: true,
        });
        oid
    }

    pub fn cancel_order(&mut self, order_id: u64) -> bool {
        for order in &mut self.orders {
            if order.order_id == order_id && order.active {
                order.active = false;
                return true;
            }
        }
        false
    }

    pub fn process_event(&mut self, event: &MarketEvent) {
        self.current_time_ns = event.timestamp_ns;

        let (affected_price, affected_side, effective_shares) = match event.kind {
            EventKind::OrderExecuted | EventKind::OrderExecutedWithPrice => {
                if let Some((p, _s, side)) = self.lob.order_info(event.order_id) {
                    (p, side, event.shares as u64)
                } else {
                    self.lob.apply_event(event);
                    return;
                }
            }
            EventKind::OrderCancel => {
                if let Some((p, _s, side)) = self.lob.order_info(event.order_id) {
                    (p, side, event.shares as u64)
                } else {
                    self.lob.apply_event(event);
                    return;
                }
            }
            EventKind::OrderDelete => {
                if let Some((p, s, side)) = self.lob.order_info(event.order_id) {
                    (p, side, s as u64)
                } else {
                    self.lob.apply_event(event);
                    return;
                }
            }
            _ => {
                self.lob.apply_event(event);
                return;
            }
        };

        for order in &mut self.orders {
            if !order.active || order.remaining == 0 {
                continue;
            }
            if event.timestamp_ns < order.live_time_ns {
                continue;
            }
            if order.price != affected_price || order.side != affected_side {
                continue;
            }

            let mut available = effective_shares;
            if order.ahead_volume > 0 {
                let dec = available.min(order.ahead_volume);
                order.ahead_volume -= dec;
                available -= dec;
            }

            if order.ahead_volume == 0 && available > 0 && order.remaining > 0 {
                match event.kind {
                    EventKind::OrderExecuted | EventKind::OrderExecutedWithPrice => {
                        let fill_qty = (available as u32).min(order.remaining);
                        if fill_qty > 0 {
                            order.remaining -= fill_qty;
                            self.fills.push(PaperFill {
                                timestamp_ns: event.timestamp_ns,
                                order_id: order.order_id,
                                price: order.price,
                                shares: fill_qty,
                                side: order.side,
                            });
                            if order.remaining == 0 {
                                order.active = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        self.lob.apply_event(event);
    }

    pub fn fills(&self) -> &[PaperFill] {
        &self.fills
    }

    pub fn active_order_count(&self) -> usize {
        self.orders.iter().filter(|o| o.active).count()
    }

    pub fn bbo(&self) -> aw_exchange::lob::Bbo {
        self.lob.bbo()
    }

    pub fn current_time_ns(&self) -> u64 {
        self.current_time_ns
    }

    pub fn order_remaining(&self, order_id: u64) -> u32 {
        for order in &self.orders {
            if order.order_id == order_id {
                return order.remaining;
            }
        }
        0
    }

    pub fn order_ahead_volume(&self, order_id: u64) -> u64 {
        for order in &self.orders {
            if order.order_id == order_id {
                return order.ahead_volume;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_add(oid: u64, price: u32, shares: u32, side: Side, ts: u64) -> MarketEvent {
        let mut e = MarketEvent::default();
        e.kind = EventKind::AddOrder;
        e.order_id = oid;
        e.price = price;
        e.shares = shares;
        e.side = side;
        e.timestamp_ns = ts;
        e
    }

    fn make_exec(oid: u64, shares: u32, ts: u64) -> MarketEvent {
        let mut e = MarketEvent::default();
        e.kind = EventKind::OrderExecuted;
        e.order_id = oid;
        e.shares = shares;
        e.timestamp_ns = ts;
        e
    }

    fn make_delete(oid: u64, ts: u64) -> MarketEvent {
        let mut e = MarketEvent::default();
        e.kind = EventKind::OrderDelete;
        e.order_id = oid;
        e.timestamp_ns = ts;
        e
    }

    #[test]
    fn paper_fill_after_queue_drain() {
        let cfg = PaperConfig {
            transit_latency_ns: 0,
            symbol: *b"AAPL    ",
        };
        let mut engine = PaperEngine::new(cfg);

        engine.process_event(&make_add(100, 18000, 200, Side::Buy, 1000));
        engine.process_event(&make_add(102, 18000, 100, Side::Buy, 1500));
        engine.process_event(&make_add(101, 18010, 500, Side::Sell, 2000));

        let oid = engine.submit_order(18000, 50, Side::Buy, 3000);
        assert_eq!(engine.order_ahead_volume(oid), 300);

        engine.process_event(&make_exec(100, 200, 4000));
        assert_eq!(engine.order_ahead_volume(oid), 100);
        assert_eq!(engine.fills().len(), 0);

        engine.process_event(&make_exec(102, 100, 5000));
        assert_eq!(engine.order_ahead_volume(oid), 0);
        assert_eq!(engine.fills().len(), 0);

        engine.process_event(&make_add(103, 18000, 80, Side::Buy, 6000));
        engine.process_event(&make_exec(103, 50, 7000));
        assert_eq!(engine.fills().len(), 1);
        assert_eq!(engine.fills()[0].shares, 50);
        assert_eq!(engine.fills()[0].price, 18000);
    }

    #[test]
    fn paper_partial_drain_then_fill() {
        let cfg = PaperConfig {
            transit_latency_ns: 0,
            symbol: *b"AAPL    ",
        };
        let mut engine = PaperEngine::new(cfg);

        engine.process_event(&make_add(100, 18000, 200, Side::Buy, 1000));
        let oid = engine.submit_order(18000, 50, Side::Buy, 2000);
        assert_eq!(engine.order_ahead_volume(oid), 200);

        engine.process_event(&make_exec(100, 180, 3000));
        assert_eq!(engine.order_ahead_volume(oid), 20);
        assert_eq!(engine.fills().len(), 0);

        engine.process_event(&make_exec(100, 50, 4000));
        assert_eq!(engine.order_ahead_volume(oid), 0);
        assert_eq!(engine.fills().len(), 1);
        assert_eq!(engine.fills()[0].shares, 30);
    }

    #[test]
    fn paper_transit_latency_delays_fill() {
        let cfg = PaperConfig {
            transit_latency_ns: 5000,
            symbol: *b"AAPL    ",
        };
        let mut engine = PaperEngine::new(cfg);

        engine.process_event(&make_add(100, 18000, 100, Side::Buy, 1000));
        engine.process_event(&make_add(101, 18010, 100, Side::Sell, 2000));

        let oid = engine.submit_order(18000, 50, Side::Buy, 3000);
        assert_eq!(engine.order_ahead_volume(oid), 100);

        engine.process_event(&make_exec(100, 100, 4000));
        assert_eq!(engine.order_ahead_volume(oid), 100);
        assert_eq!(engine.fills().len(), 0);

        engine.process_event(&make_add(102, 18000, 100, Side::Buy, 9000));
        engine.process_event(&make_exec(102, 100, 10000));
        assert_eq!(engine.order_ahead_volume(oid), 0);
    }

    #[test]
    fn paper_cancel_prevents_fill() {
        let cfg = PaperConfig {
            transit_latency_ns: 0,
            symbol: *b"AAPL    ",
        };
        let mut engine = PaperEngine::new(cfg);

        engine.process_event(&make_add(100, 18000, 100, Side::Buy, 1000));
        let oid = engine.submit_order(18000, 50, Side::Buy, 2000);
        assert!(engine.cancel_order(oid));
        assert_eq!(engine.active_order_count(), 0);

        engine.process_event(&make_exec(100, 100, 3000));
        assert_eq!(engine.fills().len(), 0);
    }

    #[test]
    fn paper_delete_decrements_ahead() {
        let cfg = PaperConfig {
            transit_latency_ns: 0,
            symbol: *b"AAPL    ",
        };
        let mut engine = PaperEngine::new(cfg);

        engine.process_event(&make_add(100, 18000, 300, Side::Buy, 1000));
        let oid = engine.submit_order(18000, 100, Side::Buy, 2000);
        assert_eq!(engine.order_ahead_volume(oid), 300);

        engine.process_event(&make_delete(100, 3000));
        assert_eq!(engine.order_ahead_volume(oid), 0);
    }
}
