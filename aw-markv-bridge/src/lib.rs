use aw_core::probes::rdtsc;
use aw_exchange::order::MarketEvent;
use aw_matching::MatchResult;

const MARKV_STUB_DECISION_CYCLES: u64 = 185;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMode {
    Stub,
    Live,
}

#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct MarkVDecision {
    pub action: u8,
    pub side: u8,
    pub confidence: u32,
    pub latency_cycles: u64,
    pub mode: BridgeMode,
    pub _pad: [u8; 40],
}

impl Default for MarkVDecision {
    fn default() -> Self {
        Self {
            action: 0,
            side: 0,
            confidence: 0,
            latency_cycles: 0,
            mode: BridgeMode::Stub,
            _pad: [0; 40],
        }
    }
}

pub const ACTION_HOLD: u8 = 0;
pub const ACTION_EXECUTE: u8 = 1;
pub const ACTION_CANCEL: u8 = 2;

pub struct MarkVBridge {
    mode: BridgeMode,
    decision_count: u64,
    total_decision_cycles: u64,
}

impl MarkVBridge {
    pub fn new_stub() -> Self {
        Self {
            mode: BridgeMode::Stub,
            decision_count: 0,
            total_decision_cycles: 0,
        }
    }

    #[inline(always)]
    pub fn decide(&mut self, event: &MarketEvent, _match_result: &MatchResult) -> MarkVDecision {
        let mut decision = MarkVDecision::default();
        decision.mode = self.mode;

        match self.mode {
            BridgeMode::Stub => {
                let ds = rdtsc();
                let mut spin = 0u64;
                while rdtsc().wrapping_sub(ds) < MARKV_STUB_DECISION_CYCLES {
                    spin = spin.wrapping_add(1);
                }
                std::hint::black_box(spin);

                decision.action = if event.shares > 0 && event.price > 0 {
                    ACTION_EXECUTE
                } else {
                    ACTION_HOLD
                };
                decision.side = event.side as u8;
                decision.confidence = 9989;
                decision.latency_cycles = MARKV_STUB_DECISION_CYCLES;
            }
            BridgeMode::Live => {
                decision.action = ACTION_HOLD;
                decision.latency_cycles = 0;
            }
        }

        self.decision_count += 1;
        self.total_decision_cycles += decision.latency_cycles;
        decision
    }

    #[inline(always)]
    pub fn mode(&self) -> BridgeMode {
        self.mode
    }

    #[inline(always)]
    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            BridgeMode::Stub => "STUB",
            BridgeMode::Live => "LIVE_MARKV",
        }
    }

    #[inline(always)]
    pub fn decision_count(&self) -> u64 {
        self.decision_count
    }

    #[inline(always)]
    pub fn avg_decision_cycles(&self) -> f64 {
        if self.decision_count == 0 {
            0.0
        } else {
            self.total_decision_cycles as f64 / self.decision_count as f64
        }
    }

    #[inline(always)]
    pub fn avg_decision_ns(&self) -> f64 {
        self.avg_decision_cycles() * 0.333
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aw_exchange::order::{EventKind, Side};

    #[test]
    fn stub_bridge_decides() {
        let mut bridge = MarkVBridge::new_stub();
        let mut event = MarketEvent::default();
        event.price = 18000;
        event.shares = 100;
        event.side = Side::Buy;
        event.kind = EventKind::AddOrder;
        let mr = MatchResult::default();
        let decision = bridge.decide(&event, &mr);
        assert_eq!(decision.action, ACTION_EXECUTE);
        assert_eq!(decision.mode, BridgeMode::Stub);
        assert_eq!(bridge.mode_label(), "STUB");
        assert!(bridge.avg_decision_ns() > 0.0);
    }

    #[test]
    fn stub_bridge_holds_zero_shares() {
        let mut bridge = MarkVBridge::new_stub();
        let event = MarketEvent::default();
        let mr = MatchResult::default();
        let decision = bridge.decide(&event, &mr);
        assert_eq!(decision.action, ACTION_HOLD);
    }

    #[test]
    fn decision_alignment() {
        assert_eq!(std::mem::align_of::<MarkVDecision>(), 64);
        assert_eq!(std::mem::size_of::<MarkVDecision>(), 64);
    }
}
