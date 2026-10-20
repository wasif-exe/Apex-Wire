use aw_exchange::order::MarketEvent;
use aw_store::event_log::{EventLogWriter, EVENT_PAYLOAD_SIZE};
use std::io::Result;

pub const SHADOW_EVENT_INGRESS: u8 = 1;
pub const SHADOW_EVENT_DECISION: u8 = 2;
pub const SHADOW_EVENT_FILL: u8 = 3;
pub const SHADOW_EVENT_REJECT: u8 = 4;
pub const SHADOW_EVENT_BBO: u8 = 5;

#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
pub struct ShadowDecision {
    pub action: u8,
    pub side: u8,
    pub venue: u8,
    pub _pad: u8,
    pub price: u32,
    pub shares: u32,
    pub latency_ns: u64,
}

pub struct ShadowLogger {
    writer: EventLogWriter,
    seq: u64,
    ingress_count: u64,
    decision_count: u64,
    fill_count: u64,
}

impl ShadowLogger {
    pub fn open(path: &str, max_bytes: usize) -> Result<Self> {
        let writer = EventLogWriter::open(path, max_bytes)?;
        Ok(Self {
            writer,
            seq: 0,
            ingress_count: 0,
            decision_count: 0,
            fill_count: 0,
        })
    }

    #[inline(always)]
    pub fn log_ingress(&mut self, event: &MarketEvent) -> Result<u64> {
        self.seq += 1;
        let event_bytes = market_event_to_bytes(event);
        self.ingress_count += 1;
        self.writer.append(
            self.seq,
            event.timestamp_ns,
            SHADOW_EVENT_INGRESS,
            &event_bytes,
        )
    }

    #[inline(always)]
    pub fn log_decision(&mut self, event: &MarketEvent, decision: &ShadowDecision) -> Result<u64> {
        self.seq += 1;
        let mut payload = [0u8; EVENT_PAYLOAD_SIZE];
        let dec_bytes = decision_to_bytes(decision);
        let copy_len = dec_bytes.len().min(EVENT_PAYLOAD_SIZE);
        payload[..copy_len].copy_from_slice(&dec_bytes[..copy_len]);
        self.decision_count += 1;
        self.writer.append(
            self.seq,
            event.timestamp_ns,
            SHADOW_EVENT_DECISION,
            &payload,
        )
    }

    #[inline(always)]
    pub fn log_fill(
        &mut self,
        timestamp_ns: u64,
        order_id: u64,
        price: u32,
        shares: u32,
    ) -> Result<u64> {
        self.seq += 1;
        let mut payload = [0u8; EVENT_PAYLOAD_SIZE];
        payload[0..8].copy_from_slice(&order_id.to_le_bytes());
        payload[8..12].copy_from_slice(&price.to_le_bytes());
        payload[12..16].copy_from_slice(&shares.to_le_bytes());
        self.fill_count += 1;
        self.writer.append(self.seq, timestamp_ns, SHADOW_EVENT_FILL, &payload)
    }

    #[inline(always)]
    pub fn log_reject(
        &mut self,
        timestamp_ns: u64,
        order_id: u64,
        reason_code: u32,
    ) -> Result<u64> {
        self.seq += 1;
        let mut payload = [0u8; EVENT_PAYLOAD_SIZE];
        payload[0..8].copy_from_slice(&order_id.to_le_bytes());
        payload[8..12].copy_from_slice(&reason_code.to_le_bytes());
        self.writer.append(self.seq, timestamp_ns, SHADOW_EVENT_REJECT, &payload)
    }

    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }

    pub fn sync(&mut self) -> Result<()> {
        self.writer.sync()
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

    pub fn total_records(&self) -> u64 {
        self.writer.record_count()
    }
}

#[inline(always)]
fn market_event_to_bytes(event: &MarketEvent) -> [u8; EVENT_PAYLOAD_SIZE] {
    let mut buf = [0u8; EVENT_PAYLOAD_SIZE];
    buf[0..8].copy_from_slice(&event.timestamp_ns.to_le_bytes());
    buf[8..16].copy_from_slice(&event.order_id.to_le_bytes());
    buf[16..20].copy_from_slice(&event.price.to_le_bytes());
    buf[20..24].copy_from_slice(&event.shares.to_le_bytes());
    buf[24..32].copy_from_slice(&event.symbol);
    buf[32] = event.kind as u8;
    buf[33] = event.side as u8;
    buf
}

#[inline(always)]
fn decision_to_bytes(dec: &ShadowDecision) -> [u8; 20] {
    let mut buf = [0u8; 20];
    buf[0] = dec.action;
    buf[1] = dec.side;
    buf[2] = dec.venue;
    buf[4..8].copy_from_slice(&dec.price.to_le_bytes());
    buf[8..12].copy_from_slice(&dec.shares.to_le_bytes());
    buf[12..20].copy_from_slice(&dec.latency_ns.to_le_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use aw_exchange::order::{EventKind, Side};

    #[test]
    fn shadow_logger_roundtrip() {
        let path = "/tmp/aw_v3_shadow_test.bin";
        let _ = std::fs::remove_file(path);
        {
            let mut logger = ShadowLogger::open(path, 64 * 1024).unwrap();
            let mut event = MarketEvent::default();
            event.kind = EventKind::AddOrder;
            event.order_id = 42;
            event.price = 18000;
            event.shares = 100;
            event.side = Side::Buy;
            event.timestamp_ns = 1_000_000;
            event.symbol = *b"AAPL    ";
            logger.log_ingress(&event).unwrap();
            let dec = ShadowDecision {
                action: 1,
                side: 1,
                venue: 0,
                _pad: 0,
                price: 18000,
                shares: 100,
                latency_ns: 500,
            };
            logger.log_decision(&event, &dec).unwrap();
            logger.log_fill(1_001_000, 42, 18000, 100).unwrap();
            logger.log_reject(1_002_000, 99, 1).unwrap();
            logger.sync().unwrap();
            assert_eq!(logger.ingress_count(), 1);
            assert_eq!(logger.decision_count(), 1);
            assert_eq!(logger.fill_count(), 1);
            assert_eq!(logger.total_records(), 4);
        }
        {
            let mut reader = aw_store::event_log::EventLogReader::open(path).unwrap();
            assert_eq!(reader.total_records(), 4);
            let r1 = reader.next_record().unwrap();
            assert_eq!(r1.event_type, SHADOW_EVENT_INGRESS);
            let r2 = reader.next_record().unwrap();
            assert_eq!(r2.event_type, SHADOW_EVENT_DECISION);
            let r3 = reader.next_record().unwrap();
            assert_eq!(r3.event_type, SHADOW_EVENT_FILL);
            let r4 = reader.next_record().unwrap();
            assert_eq!(r4.event_type, SHADOW_EVENT_REJECT);
        }
        let _ = std::fs::remove_file(path);
    }
}
