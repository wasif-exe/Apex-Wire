pub const DUP_SLOTS: usize = 65536;
#[repr(C, align(64))]
pub struct FeedArbitrator {
    bitmask: Vec<u64>,
    mask: u64,
    pub feed_a_processed: u64,
    pub feed_b_processed: u64,
    pub duplicates_dropped: u64,
    pub gaps_recovered: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedSource {
    FeedA,
    FeedB,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbitratorVerdict {
    Process,
    DuplicateDrop,
}
impl FeedArbitrator {
    pub fn new() -> Self {
        let num_u64s = DUP_SLOTS / 64;
        let mut bitmask = Vec::with_capacity(num_u64s);
        for _ in 0..num_u64s {
            bitmask.push(0);
        }
        Self {
            bitmask,
            mask: (DUP_SLOTS - 1) as u64,
            feed_a_processed: 0,
            feed_b_processed: 0,
            duplicates_dropped: 0,
            gaps_recovered: 0,
        }
    }
    #[inline(always)]
    pub fn arbitrate(&mut self, seq: u64, source: FeedSource) -> ArbitratorVerdict {
        let slot_idx = (seq & self.mask) as usize;
        let array_idx = slot_idx >> 6;
        let bit_flag = 1u64 << (slot_idx & 63);
        // SAFETY: Direct pointer dereference of contiguous bitmask allocation.
        let entry_ptr = unsafe { self.bitmask.as_mut_ptr().add(array_idx) };
        let prev = unsafe { *entry_ptr };
        if (prev & bit_flag) != 0 {
            self.duplicates_dropped += 1;
            ArbitratorVerdict::DuplicateDrop
        } else {
            unsafe {
                *entry_ptr = prev | bit_flag;
            }
            match source {
                FeedSource::FeedA => { self.feed_a_processed += 1; }
                FeedSource::FeedB => {
                    self.feed_b_processed += 1;
                    self.gaps_recovered += 1;
                }
            }
            ArbitratorVerdict::Process
        }
    }
    pub fn reset(&mut self) {
        for slot in &mut self.bitmask {
            *slot = 0;
        }
        self.feed_a_processed = 0;
        self.feed_b_processed = 0;
        self.duplicates_dropped = 0;
        self.gaps_recovered = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_feed_arbitration() {
        let mut arb = FeedArbitrator::new();
        assert_eq!(arb.arbitrate(100, FeedSource::FeedA), ArbitratorVerdict::Process);
        assert_eq!(arb.arbitrate(100, FeedSource::FeedB), ArbitratorVerdict::DuplicateDrop);
        assert_eq!(arb.arbitrate(101, FeedSource::FeedB), ArbitratorVerdict::Process);
        assert_eq!(arb.arbitrate(101, FeedSource::FeedA), ArbitratorVerdict::DuplicateDrop);
        assert_eq!(arb.duplicates_dropped, 2);
        assert_eq!(arb.gaps_recovered, 1);
    }
}
