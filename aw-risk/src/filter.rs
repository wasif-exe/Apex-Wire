use aw_exchange::order::{EventKind, MarketEvent};
use aw_core::probes::{self, StageId};
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolRiskLimits {
    pub max_order_size: u32,
    pub min_price: u32,
    pub max_price: u32,
    pub max_notional: u64,
    pub enabled: bool,
    pub _pad: [u8; 7],
}
impl Default for SymbolRiskLimits {
    #[inline(always)]
    fn default() -> Self {
        Self {
            max_order_size: 10_000,
            min_price: 1,
            max_price: 1_000_000_000,
            max_notional: 100_000_000_000,
            enabled: true,
            _pad: [0; 7],
        }
    }
}
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskVerdict {
    Pass = 0,
    RejectMaxOrderSize = 1,
    RejectMinPrice = 2,
    RejectMaxPrice = 3,
    RejectMaxNotional = 4,
    RejectSymbolDisabled = 5,
}
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, Default)]
pub struct RiskFilterStats {
    pub passed_count: u64,
    pub rejected_order_size: u64,
    pub rejected_price: u64,
    pub rejected_notional: u64,
    pub rejected_symbol: u64,
}
pub struct PreTradeRiskFilter {
    symbols: Vec<([u8; 8], SymbolRiskLimits)>,
    pub stats: RiskFilterStats,
}
impl PreTradeRiskFilter {
    pub fn new() -> Self {
        Self {
            symbols: Vec::with_capacity(256),
            stats: RiskFilterStats::default(),
        }
    }
    pub fn set_limits(&mut self, symbol: [u8; 8], limits: SymbolRiskLimits) {
        for entry in self.symbols.iter_mut() {
            if entry.0 == symbol {
                entry.1 = limits;
                return;
            }
        }
        self.symbols.push((symbol, limits));
    }
    #[inline(always)]
    pub fn get_limits(&self, symbol: &[u8; 8]) -> Option<&SymbolRiskLimits> {
        for entry in &self.symbols {
            if &entry.0 == symbol {
                return Some(&entry.1);
            }
        }
        None
    }
    #[inline(always)]
    pub fn check_order(&mut self, event: &MarketEvent) -> RiskVerdict {
        let t_begin = probes::stage_begin(StageId::XdpRiskFilter);
        if event.kind != EventKind::AddOrder && event.kind != EventKind::OrderReplace {
            self.stats.passed_count += 1;
            probes::stage_end(StageId::XdpRiskFilter, t_begin);
            return RiskVerdict::Pass;
        }
        let limits = match self.get_limits(&event.symbol) {
            Some(lim) => lim,
            None => {
                self.stats.passed_count += 1;
                probes::stage_end(StageId::XdpRiskFilter, t_begin);
                return RiskVerdict::Pass;
            }
        };
        if !limits.enabled {
            self.stats.rejected_symbol += 1;
            probes::stage_end(StageId::XdpRiskFilter, t_begin);
            return RiskVerdict::RejectSymbolDisabled;
        }
        if limits.max_order_size > 0 && event.shares > limits.max_order_size {
            self.stats.rejected_order_size += 1;
            probes::stage_end(StageId::XdpRiskFilter, t_begin);
            return RiskVerdict::RejectMaxOrderSize;
        }
        if limits.min_price > 0 && event.price < limits.min_price {
            self.stats.rejected_price += 1;
            probes::stage_end(StageId::XdpRiskFilter, t_begin);
            return RiskVerdict::RejectMinPrice;
        }
        if limits.max_price > 0 && event.price > limits.max_price {
            self.stats.rejected_price += 1;
            probes::stage_end(StageId::XdpRiskFilter, t_begin);
            return RiskVerdict::RejectMaxPrice;
        }
        let notional = (event.shares as u64).wrapping_mul(event.price as u64);
        if limits.max_notional > 0 && notional > limits.max_notional {
            self.stats.rejected_notional += 1;
            probes::stage_end(StageId::XdpRiskFilter, t_begin);
            return RiskVerdict::RejectMaxNotional;
        }
        self.stats.passed_count += 1;
        probes::stage_end(StageId::XdpRiskFilter, t_begin);
        RiskVerdict::Pass
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use aw_exchange::order::Side;
    #[test]
    fn risk_filter_rules() {
        let mut filter = PreTradeRiskFilter::new();
        let sym = *b"NVDA    ";
        filter.set_limits(sym, SymbolRiskLimits {
            max_order_size: 500,
            min_price: 10000,
            max_price: 20000,
            max_notional: 5_000_000,
            enabled: true,
            _pad: [0; 7],
        });
        let mut valid_order = MarketEvent::default();
        valid_order.kind = EventKind::AddOrder;
        valid_order.symbol = sym;
        valid_order.shares = 100;
        valid_order.price = 15000;
        valid_order.side = Side::Buy;
        assert_eq!(filter.check_order(&valid_order), RiskVerdict::Pass);
        let mut oversize_order = valid_order;
        oversize_order.shares = 1000;
        assert_eq!(filter.check_order(&oversize_order), RiskVerdict::RejectMaxOrderSize);
        let mut low_price_order = valid_order;
        low_price_order.price = 5000;
        assert_eq!(filter.check_order(&low_price_order), RiskVerdict::RejectMinPrice);
        let mut over_notional_order = valid_order;
        over_notional_order.shares = 400;
        over_notional_order.price = 19000;
        assert_eq!(filter.check_order(&over_notional_order), RiskVerdict::RejectMaxNotional);
    }
}

