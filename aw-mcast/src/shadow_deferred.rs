use aw_disruptor::Disruptor;
use aw_exchange::order::MarketEvent;
use aw_store::event_log::{EventLogWriter, EVENT_PAYLOAD_SIZE};
use std::io::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::shadow::{
    decision_to_bytes, market_event_to_bytes, SHADOW_EVENT_DECISION, SHADOW_EVENT_FILL,
    SHADOW_EVENT_INGRESS, SHADOW_EVENT_REJECT,
};

const RING_CAPACITY: usize = 65536;
const FLUSH_BATCH: u64 = 512;

#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct ShadowRecord {
    pub seq: u64,
    pub timestamp_ns: u64,
    pub event_type: u8,
    pub _pad: [u8; 7],
    pub payload: [u8; EVENT_PAYLOAD_SIZE],
}

impl Default for ShadowRecord {
    #[inline(always)]
    fn default() -> Self {
        Self {
            seq: 0,
            timestamp_ns: 0,
            event_type: 0,
            _pad: [0; 7],
            payload: [0; EVENT_PAYLOAD_SIZE],
        }
    }
}

const _: () = assert!(core::mem::size_of::<ShadowRecord>() == 64);

pub struct ShadowDeferred {
    ring: &'static Disruptor<ShadowRecord>,
    seq: u64,
    ingress_count: u64,
    decision_count: u64,
    fill_count: u64,
}

impl ShadowDeferred {
    pub fn new() -> Self {
        let ring = Box::leak(Box::new(
            Disruptor::<ShadowRecord>::new(RING_CAPACITY)
                .expect("failed to allocate shadow ring"),
        ));
        Self {
            ring,
            seq: 0,
            ingress_count: 0,
            decision_count: 0,
            fill_count: 0,
        }
    }

    #[inline(always)]
    fn publish_record(&mut self, timestamp_ns: u64, event_type: u8, payload: &[u8]) {
        self.seq += 1;
        let pub_handle = self.ring.publisher();
        let s = pub_handle.claim_spin();
        let slot = pub_handle.get_mut(s);
        slot.seq = self.seq;
        slot.timestamp_ns = timestamp_ns;
        slot.event_type = event_type;
        let len = payload.len().min(EVENT_PAYLOAD_SIZE);
        slot.payload[..len].copy_from_slice(&payload[..len]);
        pub_handle.publish(s);
    }

    #[inline(always)]
    pub fn log_ingress(&mut self, event: &MarketEvent) {
        let bytes = market_event_to_bytes(event);
        self.ingress_count += 1;
        self.publish_record(event.timestamp_ns, SHADOW_EVENT_INGRESS, &bytes);
    }

    #[inline(always)]
    pub fn log_decision(&mut self, event: &MarketEvent, decision: &crate::shadow::ShadowDecision) {
        let bytes = decision_to_bytes(decision);
        self.decision_count += 1;
        self.publish_record(event.timestamp_ns, SHADOW_EVENT_DECISION, &bytes);
    }

    #[inline(always)]
    pub fn log_fill(&mut self, timestamp_ns: u64, order_id: u64, price: u32, shares: u32) {
        let mut payload = [0u8; EVENT_PAYLOAD_SIZE];
        payload[0..8].copy_from_slice(&order_id.to_le_bytes());
        payload[8..12].copy_from_slice(&price.to_le_bytes());
        payload[12..16].copy_from_slice(&shares.to_le_bytes());
        self.fill_count += 1;
        self.publish_record(timestamp_ns, SHADOW_EVENT_FILL, &payload);
    }

    #[inline(always)]
    pub fn log_reject(&mut self, timestamp_ns: u64, order_id: u64, reason_code: u32) {
        let mut payload = [0u8; EVENT_PAYLOAD_SIZE];
        payload[0..8].copy_from_slice(&order_id.to_le_bytes());
        payload[8..12].copy_from_slice(&reason_code.to_le_bytes());
        self.publish_record(timestamp_ns, SHADOW_EVENT_REJECT, &payload);
    }

    pub fn spawn_flush_thread(
        &self,
        path: &str,
        max_bytes: usize,
        running: Arc<AtomicBool>,
    ) -> Result<std::thread::JoinHandle<u64>> {
        let path = path.to_string();
        let consumer = self.ring.consumer();
        let handle = std::thread::spawn(move || {
            let mut cons = consumer;
            let mut writer = EventLogWriter::open(&path, max_bytes)
                .expect("failed to open shadow WAL");
            let mut total_flushed: u64 = 0;

            while running.load(Ordering::Acquire) {
                let drained = cons.drain(FLUSH_BATCH, |rec| {
                    let _ = writer.append(rec.seq, rec.timestamp_ns, rec.event_type, &rec.payload);
                    total_flushed += 1;
                });
                if drained == 0 {
                    std::hint::spin_loop();
                }
            }

            let _drained = cons.drain(u64::MAX, |rec| {
                let _ = writer.append(rec.seq, rec.timestamp_ns, rec.event_type, &rec.payload);
                total_flushed += 1;
            });
            let _ = writer.sync();
            total_flushed
        });
        Ok(handle)
    }

    pub fn ingress_count(&self) -> u64 {
        self.ingress_count
    }

    pub fn decision_count(&self) -> u64 {
        self.decision_count
    }

    pub fn fill_count(&self) -> u64 {
        self.fill_count
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aw_exchange::order::{EventKind, Side};

    #[test]
    fn deferred_roundtrip() {
        let path = "/tmp/aw_shadow_deferred_test.bin";
        let _ = std::fs::remove_file(path);
        let running = Arc::new(AtomicBool::new(true));
        let mut deferred = ShadowDeferred::new();
        let flush_handle = deferred
            .spawn_flush_thread(path, 64 * 1024, running.clone())
            .unwrap();

        let mut event = MarketEvent::default();
        event.kind = EventKind::AddOrder;
        event.order_id = 42;
        event.price = 18000;
        event.shares = 100;
        event.side = Side::Buy;
        event.timestamp_ns = 1_000_000;
        event.symbol = *b"AAPL    ";

        for _ in 0..100 {
            deferred.log_ingress(&event);
        }
        deferred.log_fill(1_001_000, 42, 18000, 100);
        deferred.log_reject(1_002_000, 99, 1);

        assert_eq!(deferred.ingress_count(), 100);
        assert_eq!(deferred.fill_count(), 1);
        assert_eq!(deferred.seq(), 102);

        running.store(false, Ordering::Release);
        let flushed = flush_handle.join().unwrap();
        assert_eq!(flushed, 102);

        let reader = aw_store::event_log::EventLogReader::open(path).unwrap();
        assert_eq!(reader.total_records(), 102);
        let _ = std::fs::remove_file(path);
    }
}
