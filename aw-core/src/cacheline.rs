use std::sync::atomic::{AtomicU64, AtomicU32};
#[repr(C, align(64))]
pub struct Aligned64<T>(pub T);
impl<T> std::ops::Deref for Aligned64<T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> std::ops::DerefMut for Aligned64<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
#[repr(C, align(64))]
pub struct WorkerStats {
    pub rx_packets: AtomicU64,
    pub tx_packets: AtomicU64,
    pub rx_bytes: AtomicU64,
    pub tx_bytes: AtomicU64,
    pub dropped_packets: AtomicU64,
    pub poll_spins: AtomicU64,
    _pad: [u8; 16],
}
impl Default for WorkerStats {
    fn default() -> Self {
        Self {
            rx_packets: AtomicU64::new(0),
            tx_packets: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            tx_bytes: AtomicU64::new(0),
            dropped_packets: AtomicU64::new(0),
            poll_spins: AtomicU64::new(0),
            _pad: [0; 16],
        }
    }
}
#[repr(C, align(64))]
pub struct CachePaddedRingIndices {
    pub prod: AtomicU32,
    _pad1: [u8; 60],
    pub cons: AtomicU32,
    _pad2: [u8; 60],
}
impl Default for CachePaddedRingIndices {
    fn default() -> Self {
        Self {
            prod: AtomicU32::new(0),
            _pad1: [0; 60],
            cons: AtomicU32::new(0),
            _pad2: [0; 60],
        }
    }
}
#[inline(always)]
pub fn cpu_relax() {
    #[cfg(target_arch = "x86_64")]
    {
        core::arch::x86_64::_mm_pause();
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        core::hint::spin_loop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cacheline_alignments() {
        assert_eq!(std::mem::align_of::<WorkerStats>(), 64);
        assert_eq!(std::mem::size_of::<WorkerStats>() % 64, 0);
        assert_eq!(std::mem::align_of::<CachePaddedRingIndices>(), 64);
        assert_eq!(std::mem::size_of::<CachePaddedRingIndices>() % 64, 0);
        assert_eq!(std::mem::align_of::<Aligned64<u64>>(), 64);
        assert_eq!(std::mem::size_of::<Aligned64<u64>>(), 64);
    }
}

