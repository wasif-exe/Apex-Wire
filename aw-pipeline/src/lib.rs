pub mod strategy;
pub use strategy::{Strategy, StrategyRegistry, Decision, Action, PassthroughStrategy, MomentumStrategy};
use aw_core::probes::{self, StageId, rdtsc};
use aw_disruptor::{Consumer, Disruptor, Publisher};
use aw_exchange::lob::LimitOrderBook;
use aw_exchange::order::MarketEvent;
use aw_matching::MatchingEngine;
use aw_sor::router::SmartOrderRouter;
use aw_vfio::device::VfioDevice;
use aw_vfio::ring::VfioTxRing;

const RING_CAPACITY: usize = 4096;
const BAR0_SIZE: usize = 4096;
const DOORBELL_OFFSET: usize = 16;

#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct PipelineResult {
    pub fill_count: u32,
    pub routed: bool,
    pub venue: u8,
    pub egress_ok: bool,
    pub risk_pass: bool,
    pub decision_cycles: u64,
    pub latency_cycles: u64,
    pub _pad: [u8; 40],
}

impl Default for PipelineResult {
    #[inline(always)]
    fn default() -> Self {
        // SAFETY: PipelineResult is repr(C) with all-zero-valid primitive fields.
        unsafe { std::mem::zeroed() }
    }
}

pub struct Pipeline {
    publisher: Publisher<'static, MarketEvent>,
    consumer: Consumer<'static, MarketEvent>,
    matching: MatchingEngine,
    lob: LimitOrderBook,
    sor: SmartOrderRouter,
    vfio: VfioDevice,
    tx_ring: VfioTxRing,
    disruptor: Disruptor<MarketEvent>,
}

unsafe impl Send for Pipeline {}

impl Pipeline {
    pub fn new(symbol: [u8; 8]) -> anyhow::Result<Self> {
        let disruptor = Disruptor::<MarketEvent>::new(RING_CAPACITY)?;
        // SAFETY: Publisher and Consumer reference Sequences inside the Disruptor

        let publisher = unsafe {
            std::mem::transmute::<Publisher<'_, MarketEvent>, Publisher<'static, MarketEvent>>(
                disruptor.publisher(),
            )
        };
        // SAFETY: Same lifetime extension justification as publisher above.
        let consumer = unsafe {
            std::mem::transmute::<Consumer<'_, MarketEvent>, Consumer<'static, MarketEvent>>(
                disruptor.consumer(),
            )
        };
        let vfio = VfioDevice::open_mock(BAR0_SIZE)?;
        let tx_ring = VfioTxRing::new(&vfio, DOORBELL_OFFSET)?;
        Ok(Self {
            publisher,
            consumer,
            matching: MatchingEngine::new(symbol),
            lob: LimitOrderBook::new(symbol),
            sor: SmartOrderRouter::new(symbol),
            vfio,
            tx_ring,
            disruptor,
        })
    }

    #[inline(always)]
    pub fn process_event(&mut self, event: &MarketEvent) -> PipelineResult {
        let t0 = rdtsc();
        let mut result = PipelineResult::default();

        let t_risk = probes::stage_begin(StageId::PipelineRisk);
        result.risk_pass = (event.price | event.shares) != 0 && event.shares <= 1_000_000;
        probes::stage_end(StageId::PipelineRisk, t_risk);

        if !result.risk_pass {
            result.latency_cycles = rdtsc().wrapping_sub(t0);
            return result;
        }

        self.publisher.try_publish_with(|slot| *slot = *event);

        let consumed = if self.consumer.has_next() {
            let seq = self.consumer.next_seq();
            let val = self.consumer.read(seq);
            self.consumer.advance(seq);
            val
        } else {
            *event
        };

        self.lob.apply_event(&consumed);

        let t_match = probes::stage_begin(StageId::PipelineMatch);
        let mr = self.matching.match_order(
            consumed.price,
            consumed.shares,
            consumed.side,
            consumed.timestamp_ns,
        );
        result.fill_count = mr.fill_count;
        probes::stage_end(StageId::PipelineMatch, t_match);

        let t_route = probes::stage_begin(StageId::PipelineRoute);
        let child = self.sor.route_order(consumed.side, consumed.shares);
        result.routed = true;
        result.venue = child.target_venue;
        probes::stage_end(StageId::PipelineRoute, t_route);

        let t_egress = probes::stage_begin(StageId::PipelineEgress);
        let mut pkt = [0u8; 16];
        let hdr = (child.price as u64) | ((child.shares as u64) << 32);
        pkt[0..8].copy_from_slice(&hdr.to_le_bytes());
        pkt[8] = child.target_venue;
        pkt[9] = child.side as u8;
        result.egress_ok = self.tx_ring.transmit(&self.vfio, &pkt);
        if self.vfio.is_mock {
            self.tx_ring.head = self.tx_ring.tail;
        }
        probes::stage_end(StageId::PipelineEgress, t_egress);

        result.latency_cycles = rdtsc().wrapping_sub(t0);
        result
    }

    #[inline(always)]
    pub fn vfio_is_mock(&self) -> bool {
        self.vfio.is_mock
    }

    #[inline(always)]
    pub fn ring_capacity(&self) -> u64 {
        self.disruptor.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aw_exchange::order::{EventKind, Side};

    #[test]
    fn pipeline_process_single_event() {
        let mut pipeline = Pipeline::new(*b"AAPL    ").unwrap();
        let mut event = MarketEvent::default();
        event.kind = EventKind::AddOrder;
        event.price = 18000;
        event.shares = 100;
        event.side = Side::Buy;
        event.timestamp_ns = 1000;
        let result = pipeline.process_event(&event);
        assert!(result.risk_pass);
        assert!(result.routed);
        assert!(result.egress_ok);
        assert!(result.latency_cycles > 0);
    }

    #[test]
    fn pipeline_rejects_zero_price() {
        let mut pipeline = Pipeline::new(*b"AAPL    ").unwrap();
        let event = MarketEvent::default();
        let result = pipeline.process_event(&event);
        assert!(!result.risk_pass);
    }

    #[test]
    fn pipeline_result_alignment() {
        assert_eq!(std::mem::align_of::<PipelineResult>(), 64);
        assert_eq!(std::mem::size_of::<PipelineResult>(), 64);
    }
}

