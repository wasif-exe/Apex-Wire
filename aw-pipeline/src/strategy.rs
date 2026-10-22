use aw_exchange::lob::LimitOrderBook;
use aw_exchange::order::MarketEvent;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Buy,
    Sell,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub struct Decision {
    pub action: Action,
    pub price: u32,
    pub shares: u32,
    pub confidence: u16,
}

impl Default for Decision {
    #[inline(always)]
    fn default() -> Self {
        Self {
            action: Action::None,
            price: 0,
            shares: 0,
            confidence: 0,
        }
    }
}

pub trait Strategy: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> u32;
    fn decide(&self, event: &MarketEvent, lob: &LimitOrderBook) -> Decision;
    fn on_fill(&self, _event: &MarketEvent) {}
    fn reset(&self) {}
}

pub struct PassthroughStrategy;

impl Strategy for PassthroughStrategy {
    fn name(&self) -> &'static str { "passthrough" }
    fn version(&self) -> u32 { 1 }
    fn decide(&self, _event: &MarketEvent, _lob: &LimitOrderBook) -> Decision {
        Decision::default()
    }
}

pub struct MomentumStrategy {
    pub lookback: usize,
    pub threshold: u32,
}

impl Strategy for MomentumStrategy {
    fn name(&self) -> &'static str { "momentum" }
    fn version(&self) -> u32 { 1 }
    fn decide(&self, event: &MarketEvent, lob: &LimitOrderBook) -> Decision {
        let bbo = lob.bbo();
        if bbo.bid_price == 0 || bbo.ask_price == 0 {
            return Decision::default();
        }
        let mid = (bbo.bid_price as u64 + bbo.ask_price as u64) / 2;
        let spread = bbo.ask_price - bbo.bid_price;
        if spread > self.threshold {
            return Decision::default();
        }
        if event.price as u64 > mid + spread as u64 / 2 {
            Decision {
                action: Action::Buy,
                price: bbo.ask_price,
                shares: 100,
                confidence: 500,
            }
        } else if (event.price as u64) < mid - spread as u64 / 2 {
            Decision {
                action: Action::Sell,
                price: bbo.bid_price,
                shares: 100,
                confidence: 500,
            }
        } else {
            Decision::default()
        }
    }
}

pub struct StrategyRegistry {
    current: RwLock<Arc<dyn Strategy>>,
    swap_count: AtomicU64,
    decision_count: AtomicU64,
}

impl StrategyRegistry {
    pub fn new(initial: Arc<dyn Strategy>) -> Self {
        Self {
            current: RwLock::new(initial),
            swap_count: AtomicU64::new(0),
            decision_count: AtomicU64::new(0),
        }
    }

    pub fn default_registry() -> Self {
        Self::new(Arc::new(PassthroughStrategy))
    }

    #[inline]
    pub fn decide(&self, event: &MarketEvent, lob: &LimitOrderBook) -> Decision {
        let guard = self.current.read().unwrap();
        let d = guard.decide(event, lob);
        self.decision_count.fetch_add(1, Ordering::Relaxed);
        d
    }

    pub fn swap(&self, new_strategy: Arc<dyn Strategy>) {
        let mut guard = self.current.write().unwrap();
        *guard = new_strategy;
        self.swap_count.fetch_add(1, Ordering::Release);
    }

    pub fn current_name(&self) -> String {
        let guard = self.current.read().unwrap();
        guard.name().to_string()
    }

    pub fn current_version(&self) -> u32 {
        let guard = self.current.read().unwrap();
        guard.version()
    }

    pub fn swap_count(&self) -> u64 {
        self.swap_count.load(Ordering::Acquire)
    }

    pub fn decision_count(&self) -> u64 {
        self.decision_count.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aw_exchange::order::{EventKind, Side};

    #[test]
    fn passthrough_returns_none() {
        let s = PassthroughStrategy;
        let lob = LimitOrderBook::new(*b"AAPL    ");
        let ev = MarketEvent::default();
        let d = s.decide(&ev, &lob);
        assert_eq!(d.action, Action::None);
    }

    #[test]
    fn registry_swap() {
        let reg = StrategyRegistry::default_registry();
        assert_eq!(reg.current_name(), "passthrough");
        assert_eq!(reg.swap_count(), 0);

        reg.swap(Arc::new(MomentumStrategy {
            lookback: 10,
            threshold: 100,
        }));
        assert_eq!(reg.current_name(), "momentum");
        assert_eq!(reg.swap_count(), 1);
    }

    #[test]
    fn registry_concurrent_read() {
        let reg = Arc::new(StrategyRegistry::default_registry());
        let ev = MarketEvent {
            kind: EventKind::AddOrder,
            price: 18000,
            shares: 100,
            side: Side::Buy,
            ..Default::default()
        };

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let r = reg.clone();
                let e = ev;
                std::thread::spawn(move || {
                    let lob = LimitOrderBook::new(*b"AAPL    ");
                    for _ in 0..1000 {
                        let _ = r.decide(&e, &lob);
                    }
                })
            })
            .collect();

        reg.swap(Arc::new(MomentumStrategy {
            lookback: 5,
            threshold: 50,
        }));

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(reg.decision_count(), 4000);
        assert_eq!(reg.swap_count(), 1);
    }
}
