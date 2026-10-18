use std::sync::atomic::{fence, Ordering};
use aw_disruptor::hugepage::HugeAllocation;
use crate::device::VfioDevice;
pub const DESC_RING_SIZE: usize = 2048;
pub const PACKET_BUF_SIZE: usize = 2048;
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Default)]
pub struct PciDmaDescriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub status: u16,
}
pub struct VfioRxRing {
    pub descriptors: HugeAllocation<PciDmaDescriptor>,
    pub buffers: HugeAllocation<u8>,
    pub head: u32,
    pub tail: u32,
    pub mask: u32,
    pub doorbell_offset: usize,
}
unsafe impl Send for VfioRxRing {}
impl VfioRxRing {
    pub fn new(device: &VfioDevice, doorbell_offset: usize) -> anyhow::Result<Self> {
        let descriptors = HugeAllocation::<PciDmaDescriptor>::alloc(DESC_RING_SIZE)?;
        let buffers = HugeAllocation::<u8>::alloc(DESC_RING_SIZE * PACKET_BUF_SIZE)?;
        device.map_dma(descriptors.ptr as *mut u8, descriptors.ptr as u64, descriptors.byte_size)?;
        device.map_dma(buffers.ptr, buffers.ptr as u64, buffers.byte_size)?;
        // SAFETY: Initializing Rx DMA descriptor ring with physical buffer IOVA addresses.
        unsafe {
            for i in 0..DESC_RING_SIZE {
                let desc_ptr = descriptors.ptr.add(i);
                let buf_addr = buffers.ptr.add(i * PACKET_BUF_SIZE) as u64;
                (*desc_ptr).addr = buf_addr;
                (*desc_ptr).len = PACKET_BUF_SIZE as u32;
                (*desc_ptr).status = 0;
            }
        }
        Ok(Self {
            descriptors,
            buffers,
            head: 0,
            tail: (DESC_RING_SIZE - 1) as u32,
            mask: (DESC_RING_SIZE - 1) as u32,
            doorbell_offset,
        })
    }
    #[inline(always)]
    pub fn poll_rx<F>(&mut self, device: &VfioDevice, mut on_pkt: F) -> usize
    where
        F: FnMut(&[u8]),
    {
        let mut processed = 0;
        let idx = (self.head & self.mask) as usize;
        let desc_ptr = unsafe { self.descriptors.ptr.add(idx) };
        // SAFETY: Volatile read of Hardware Descriptor status byte.
        let status = unsafe { std::ptr::read_volatile(&(*desc_ptr).status) };
        if (status & 1) != 0 {
            // SAFETY: Direct memory slice access from DMA physical buffer.
            let len = unsafe { std::ptr::read_volatile(&(*desc_ptr).len) } as usize;
            let pkt_ptr = unsafe { self.buffers.ptr.add(idx * PACKET_BUF_SIZE) };
            let slice = unsafe { std::slice::from_raw_parts(pkt_ptr, len) };
            on_pkt(slice);
            // SAFETY: Clearing hardware status byte and recycling descriptor.
            unsafe {
                std::ptr::write_volatile(&mut (*desc_ptr).status, 0);
            }
            self.head = self.head.wrapping_add(1);
            self.tail = self.tail.wrapping_add(1);
            processed += 1;
            self.ring_doorbell(device);
        }
        processed
    }
    #[inline(always)]
    pub fn ring_doorbell(&self, device: &VfioDevice) {
        fence(Ordering::Release);
        if !device.bar0_ptr.is_null() && self.doorbell_offset < device.bar0_size {
            // SAFETY: Direct MMIO write to physical NIC BAR0 doorbell register.
            unsafe {
                let db_ptr = device.bar0_ptr.add(self.doorbell_offset) as *mut u32;
                std::ptr::write_volatile(db_ptr, self.tail);
            }
        }
    }
}
pub struct VfioTxRing {
    pub descriptors: HugeAllocation<PciDmaDescriptor>,
    pub buffers: HugeAllocation<u8>,
    pub head: u32,
    pub tail: u32,
    pub mask: u32,
    pub doorbell_offset: usize,
}
unsafe impl Send for VfioTxRing {}
impl VfioTxRing {
    pub fn new(device: &VfioDevice, doorbell_offset: usize) -> anyhow::Result<Self> {
        let descriptors = HugeAllocation::<PciDmaDescriptor>::alloc(DESC_RING_SIZE)?;
        let buffers = HugeAllocation::<u8>::alloc(DESC_RING_SIZE * PACKET_BUF_SIZE)?;
        device.map_dma(descriptors.ptr as *mut u8, descriptors.ptr as u64, descriptors.byte_size)?;
        device.map_dma(buffers.ptr, buffers.ptr as u64, buffers.byte_size)?;
        Ok(Self {
            descriptors,
            buffers,
            head: 0,
            tail: 0,
            mask: (DESC_RING_SIZE - 1) as u32,
            doorbell_offset,
        })
    }
    #[inline(always)]
    pub fn transmit(&mut self, device: &VfioDevice, packet: &[u8]) -> bool {
        if self.tail.wrapping_sub(self.head) >= DESC_RING_SIZE as u32 {
            return false;
        }
        let idx = (self.tail & self.mask) as usize;
        let buf_ptr = unsafe { self.buffers.ptr.add(idx * PACKET_BUF_SIZE) };
        let desc_ptr = unsafe { self.descriptors.ptr.add(idx) };
        let copy_len = packet.len().min(PACKET_BUF_SIZE);
        // SAFETY: Non-overlapping memory copy into physical DMA Tx memory buffer segment.
        unsafe {
            std::ptr::copy_nonoverlapping(packet.as_ptr(), buf_ptr, copy_len);
            (*desc_ptr).addr = buf_ptr as u64;
            (*desc_ptr).len = copy_len as u32;
            std::ptr::write_volatile(&mut (*desc_ptr).status, 1);
        }
        self.tail = self.tail.wrapping_add(1);
        self.ring_doorbell(device);
        true
    }
    #[inline(always)]
    pub fn ring_doorbell(&self, device: &VfioDevice) {
        fence(Ordering::Release);
        if !device.bar0_ptr.is_null() && self.doorbell_offset < device.bar0_size {
            // SAFETY: Direct MMIO write to physical NIC BAR0 Tx doorbell register.
            unsafe {
                let db_ptr = device.bar0_ptr.add(self.doorbell_offset) as *mut u32;
                std::ptr::write_volatile(db_ptr, self.tail);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_vfio_tx_ring() {
        let dev = VfioDevice::open_mock(4096).unwrap();
        let mut tx = VfioTxRing::new(&dev, 16).unwrap();
        let data = b"HEAVY_SYSTEMS_PCI_DMA_PACKET";
        assert!(tx.transmit(&dev, data));
        assert_eq!(tx.tail, 1);
    }
}
