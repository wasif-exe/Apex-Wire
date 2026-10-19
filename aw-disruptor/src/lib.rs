pub mod hugepage;

use std::sync::atomic::{AtomicU64, Ordering};
use hugepage::{HugeAllocation, HugePageKind};
use aw_core::cacheline::cpu_relax;

const INITIAL_SEQUENCE: u64 = u64::MAX;

#[repr(C, align(64))]
pub struct Sequence {
    value: AtomicU64,
    _pad: [u8; 56],
}

impl Sequence {
    #[inline(always)]
    pub fn new(val: u64) -> Self {
        Self {
            value: AtomicU64::new(val),
            _pad: [0; 56],
        }
    }

    #[inline(always)]
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn set(&self, val: u64) {
        self.value.store(val, Ordering::Release);
    }

    #[inline(always)]
    pub fn get_relaxed(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn set_relaxed(&self, val: u64) {
        self.value.store(val, Ordering::Relaxed);
    }
}

#[repr(C)]
pub struct Disruptor<T: Copy + Default> {
    cursor: Sequence,
    claim_cursor: Sequence,
    gating: Sequence,
    buffer: HugeAllocation<T>,
    capacity: u64,
    mask: u64,
}

unsafe impl<T: Copy + Default + Send> Send for Disruptor<T> {}
unsafe impl<T: Copy + Default + Send + Sync> Sync for Disruptor<T> {}

impl<T: Copy + Default> Disruptor<T> {
    pub fn new(capacity: usize) -> anyhow::Result<Self> {
        assert!(capacity.is_power_of_two());
        let cap = capacity as u64;
        let mask = cap - 1;
        let buffer = HugeAllocation::<T>::alloc(capacity)?;
        // SAFETY: Initializing all ring buffer slots to T::default() within allocated bounds.
        unsafe {
            for i in 0..capacity {
                buffer.ptr.add(i).write(T::default());
            }
        }
        Ok(Self {
            cursor: Sequence::new(INITIAL_SEQUENCE),
            claim_cursor: Sequence::new(INITIAL_SEQUENCE),
            gating: Sequence::new(INITIAL_SEQUENCE),
            buffer,
            capacity: cap,
            mask,
        })
    }

    pub fn page_kind(&self) -> HugePageKind {
        self.buffer.kind
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn publisher(&self) -> Publisher<'_, T> {
        Publisher {
            buffer: self.buffer.ptr,
            mask: self.mask,
            capacity: self.capacity,
            cursor: &self.cursor,
            claim_cursor: &self.claim_cursor,
            gating: &self.gating,
        }
    }

    pub fn consumer(&self) -> Consumer<'_, T> {
        Consumer {
            buffer: self.buffer.ptr,
            mask: self.mask,
            cursor: &self.cursor,
            gating: &self.gating,
            local_seq: INITIAL_SEQUENCE.wrapping_add(1),
        }
    }
}

pub struct Publisher<'a, T: Copy + Default> {
    buffer: *mut T,
    mask: u64,
    capacity: u64,
    cursor: &'a Sequence,
    claim_cursor: &'a Sequence,
    gating: &'a Sequence,
}

unsafe impl<'a, T: Copy + Default + Send> Send for Publisher<'a, T> {}

impl<'a, T: Copy + Default> Publisher<'a, T> {
    #[inline(always)]
    pub fn try_claim(&self) -> Option<u64> {
        let current = self.claim_cursor.get_relaxed();
        if current.wrapping_sub(self.gating.get()) >= self.capacity {
            return None;
        }
        let next = current.wrapping_add(1);
        self.claim_cursor.set_relaxed(next);
        Some(next)
    }

    #[inline(always)]
    pub fn claim_spin(&self) -> u64 {
        loop {
            if let Some(seq) = self.try_claim() {
                return seq;
            }
            cpu_relax();
        }
    }

    #[inline(always)]
    pub fn get_mut(&self, seq: u64) -> &mut T {
        // SAFETY: seq & mask yields valid index within allocated ring buffer.
        unsafe { &mut *self.buffer.add((seq & self.mask) as usize) }
    }

    #[inline(always)]
    pub fn publish(&self, seq: u64) {
        self.cursor.set(seq);
    }

    #[inline(always)]
    pub fn try_publish_with<F: FnOnce(&mut T)>(&self, f: F) -> bool {
        match self.try_claim() {
            Some(seq) => {
                let slot = self.get_mut(seq);
                f(slot);
                self.publish(seq);
                true
            }
            None => false,
        }
    }
}

pub struct Consumer<'a, T: Copy + Default> {
    buffer: *mut T,
    mask: u64,
    cursor: &'a Sequence,
    gating: &'a Sequence,
    local_seq: u64,
}

unsafe impl<'a, T: Copy + Default + Send> Send for Consumer<'a, T> {}

impl<'a, T: Copy + Default> Consumer<'a, T> {
    #[inline(always)]
    pub fn available(&self) -> u64 {
        self.cursor.get()
    }

    #[inline(always)]
    pub fn has_next(&self) -> bool {
        self.cursor.get() >= self.local_seq
    }

    #[inline(always)]
    pub fn next_seq(&self) -> u64 {
        self.local_seq
    }

    #[inline(always)]
    pub fn read(&self, seq: u64) -> T {
        // SAFETY: seq & mask yields valid index within allocated ring buffer.
        unsafe { *self.buffer.add((seq & self.mask) as usize) }
    }

    #[inline(always)]
    pub fn advance(&mut self, seq: u64) {
        self.local_seq = seq.wrapping_add(1);
        self.gating.set(seq);
    }

    #[inline(always)]
    pub fn spin_wait(&self) -> u64 {
        loop {
            let avail = self.cursor.get();
            if avail >= self.local_seq {
                return self.local_seq;
            }
            cpu_relax();
        }
    }

    #[inline(always)]
    pub fn drain<F: FnMut(T)>(&mut self, max: u64, mut f: F) -> u64 {
        let available = self.cursor.get();
        let mut seq = self.local_seq;
        if seq > available {
            return 0;
        }
        let end = available.min(seq.wrapping_add(max).wrapping_sub(1));
        let buf = self.buffer;
        let m = self.mask;
        while seq <= end {
            // SAFETY: seq & m yields valid index within allocated ring buffer.
            f(unsafe { *buf.add((seq & m) as usize) });
            seq = seq.wrapping_add(1);
        }
        let count = seq.wrapping_sub(self.local_seq);
        self.local_seq = seq;
        self.gating.set(seq.wrapping_sub(1));
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C, align(64))]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestEvent {
        pub price: u64,
        pub qty: u64,
        pub seq_id: u64,
        pub side: u8,
        pub _pad1: [u8; 32],
        pub _pad2: [u8; 7],
    }

    impl Default for TestEvent {
        fn default() -> Self {
            Self {
                price: 0,
                qty: 0,
                seq_id: 0,
                side: 0,
                _pad1: [0; 32],
                _pad2: [0; 7],
            }
        }
    }

    #[test]
    fn spsc_publish_consume() {
        let d = Disruptor::<TestEvent>::new(1024).unwrap();
        let pub_handle = d.publisher();
        let mut cons = d.consumer();
        for i in 0..100u64 {
            let seq = pub_handle.claim_spin();
            let slot = pub_handle.get_mut(seq);
            slot.price = i * 100;
            slot.qty = 1000;
            slot.seq_id = i;
            slot.side = 1;
            pub_handle.publish(seq);
        }
        let mut received = 0u64;
        let drained = cons.drain(100, |ev| {
            assert_eq!(ev.price, received * 100);
            assert_eq!(ev.seq_id, received);
            received += 1;
        });
        assert_eq!(drained, 100);
        assert_eq!(received, 100);
    }

    #[test]
    fn backpressure_full_ring() {
        let d = Disruptor::<TestEvent>::new(4).unwrap();
        let pub_handle = d.publisher();
        for _ in 0..4 {
            assert!(pub_handle.try_claim().is_some());
        }
        assert!(pub_handle.try_claim().is_none());
    }

    #[test]
    fn sequence_alignment() {
        assert_eq!(std::mem::align_of::<Sequence>(), 64);
        assert_eq!(std::mem::size_of::<Sequence>(), 64);
    }
}

