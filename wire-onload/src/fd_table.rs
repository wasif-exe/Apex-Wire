use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
pub const MAX_INTERCEPT_FDS: usize = 1024;
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdType {
    KernelSystem = 0,
    WireIntercepted = 1,
}
#[repr(C, align(64))]
pub struct FdEntry {
    pub is_active: AtomicBool,
    pub fd_type: AtomicU8,
    pub local_port: u16,
    pub remote_port: u16,
    pub tx_buffer: [u8; 2048],
    pub rx_buffer: [u8; 2048],
    pub tx_len: usize,
    pub rx_len: usize,
}
impl Default for FdEntry {
    fn default() -> Self {
        Self {
            is_active: AtomicBool::new(false),
            fd_type: AtomicU8::new(FdType::KernelSystem as u8),
            local_port: 0,
            remote_port: 0,
            tx_buffer: [0u8; 2048],
            rx_buffer: [0u8; 2048],
            tx_len: 0,
            rx_len: 0,
        }
    }
}
pub struct FdTable {
    entries: Vec<FdEntry>,
    mask: usize,
}
impl FdTable {
    pub fn new() -> Self {
        let mut entries = Vec::with_capacity(MAX_INTERCEPT_FDS);
        for _ in 0..MAX_INTERCEPT_FDS {
            entries.push(FdEntry::default());
        }
        Self {
            entries,
            mask: MAX_INTERCEPT_FDS - 1,
        }
    }
    #[inline(always)]
    pub fn register(&mut self, fd: i32, fd_type: FdType) {
        if fd >= 0 {
            let idx = (fd as usize) & self.mask;
            let entry = &mut self.entries[idx];
            entry.fd_type.store(fd_type as u8, Ordering::Release);
            entry.is_active.store(true, Ordering::Release);
        }
    }
    #[inline(always)]
    pub fn is_intercepted(&self, fd: i32) -> bool {
        if fd < 0 {
            return false;
        }
        let idx = (fd as usize) & self.mask;
        let entry = &self.entries[idx];
        entry.is_active.load(Ordering::Acquire)
            && entry.fd_type.load(Ordering::Acquire) == (FdType::WireIntercepted as u8)
    }
    #[inline(always)]
    pub fn unregister(&mut self, fd: i32) {
        if fd >= 0 {
            let idx = (fd as usize) & self.mask;
            let entry = &mut self.entries[idx];
            entry.is_active.store(false, Ordering::Release);
            entry.fd_type.store(FdType::KernelSystem as u8, Ordering::Release);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_fd_table() {
        let mut table = FdTable::new();
        table.register(42, FdType::WireIntercepted);
        assert!(table.is_intercepted(42));
        assert!(!table.is_intercepted(43));
        table.unregister(42);
        assert!(!table.is_intercepted(42));
    }
}
