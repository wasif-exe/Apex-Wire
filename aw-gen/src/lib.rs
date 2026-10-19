use aw_exchange::order::{EventKind, MarketEvent, Side};
use aw_pipeline::Pipeline;

const MAX_LATENCY_SAMPLES: usize = 1_000_000;

#[derive(Clone, Copy, Default)]
pub struct StressReport {
    pub flow_count: u32,
    pub total_events: u64,
    pub accepted_events: u64,
    pub dropped_events: u64,
    pub p50_ns: f64,
    pub p99_ns: f64,
    pub p99_9_ns: f64,
    pub p99_99_ns: f64,
    pub drop_rate: f64,
    pub pass: bool,
}

#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct FlowState {
    symbol: [u8; 8],
    price_base: u32,
    seq: u64,
    side_toggle: u8,
    _pad: [u8; 39],
}

impl Default for FlowState {
    fn default() -> Self {
        Self {
            symbol: [0; 8],
            price_base: 0,
            seq: 0,
            side_toggle: 0,
            _pad: [0; 39],
        }
    }
}

pub struct StressGenerator {
    flows: Vec<FlowState>,
    latency_buf: Vec<u64>,
}

impl StressGenerator {
    pub fn new(flow_count: usize) -> Self {
        let mut flows = Vec::with_capacity(flow_count);
        for i in 0..flow_count {
            let sym = format!("SYM{:05}", i);
            let mut symbol = [0u8; 8];
            let bytes = sym.as_bytes();
            let len = bytes.len().min(8);
            symbol[..len].copy_from_slice(&bytes[..len]);
            flows.push(FlowState {
                symbol,
                price_base: 18000 + (i as u32 % 1000),
                seq: 0,
                side_toggle: 0,
                _pad: [0; 39],
            });
        }
        Self {
            flows,
            latency_buf: Vec::with_capacity(MAX_LATENCY_SAMPLES),
        }
    }

    pub fn run(&mut self, events_per_flow: u64) -> StressReport {
        let mut pipeline = match Pipeline::new(*b"STRESS  ") {
            Ok(p) => p,
            Err(_) => {
                return StressReport {
                    flow_count: self.flows.len() as u32,
                    total_events: 0,
                    accepted_events: 0,
                    dropped_events: 0,
                    p50_ns: 0.0,
                    p99_ns: 0.0,
                    p99_9_ns: 0.0,
                    p99_99_ns: 0.0,
                    drop_rate: 1.0,
                    pass: false,
                };
            }
        };

        let total_events = self.flows.len() as u64 * events_per_flow;
        let mut accepted = 0u64;
        let mut dropped = 0u64;
        self.latency_buf.clear();

        for flow in &mut self.flows {
            for _ in 0..events_per_flow {
                flow.seq += 1;
                flow.side_toggle ^= 1;
                let side = if flow.side_toggle == 0 {
                    Side::Buy
                } else {
                    Side::Sell
                };
                let price = flow.price_base + (flow.seq as u32 % 100);
                let mut event = MarketEvent::default();
                event.kind = EventKind::AddOrder;
                event.price = price;
                event.shares = 100;
                event.side = side;
                event.timestamp_ns = flow.seq;
                event.symbol = flow.symbol;

                let result = pipeline.process_event(&event);
                if result.risk_pass && result.routed && result.egress_ok {
                    accepted += 1;
                    if self.latency_buf.len() < MAX_LATENCY_SAMPLES {
                        let ns = result.latency_cycles as f64 * 0.333;
                        self.latency_buf.push(ns as u64);
                    }
                } else {
                    dropped += 1;
                }
            }
        }

        self.latency_buf.sort_unstable();
        let len = self.latency_buf.len();
        let p50 = if len > 0 {
            self.latency_buf[len / 2] as f64
        } else {
            0.0
        };
        let p99 = if len > 0 {
            self.latency_buf[(len as f64 * 0.99) as usize] as f64
        } else {
            0.0
        };
        let p99_9 = if len > 0 {
            let idx = ((len as f64 * 0.999) as usize).min(len - 1);
            self.latency_buf[idx] as f64
        } else {
            0.0
        };
        let p99_99 = if len > 0 {
            let idx = ((len as f64 * 0.9999) as usize).min(len - 1);
            self.latency_buf[idx] as f64
        } else {
            0.0
        };

        let drop_rate = if total_events > 0 {
            dropped as f64 / total_events as f64
        } else {
            1.0
        };

        StressReport {
            flow_count: self.flows.len() as u32,
            total_events,
            accepted_events: accepted,
            dropped_events: dropped,
            p50_ns: p50,
            p99_ns: p99,
            p99_9_ns: p99_9,
            p99_99_ns: p99_99,
            drop_rate,
            pass: drop_rate == 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_generator_small_run() {
        let mut gen = StressGenerator::new(4);
        let report = gen.run(100);
        assert_eq!(report.flow_count, 4);
        assert_eq!(report.total_events, 400);
        assert!(report.accepted_events > 0);
    }
}

