pub const CACHE_LINE_SIZE: usize = 64;
pub const DDIO_DESCRIPTOR_ALIGNMENT: usize = 64;
#[repr(C, align(64))]
pub struct DdioAlignedBuffer<const N: usize> {
    pub data: [u8; N],
}
impl<const N: usize> Default for DdioAlignedBuffer<N> {
    fn default() -> Self {
        Self {
            data: [0u8; N],
        }
    }
}
#[inline(always)]
pub fn prefetch_l3_ddio(addr: *const u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_prefetch(addr as *const i8, core::arch::x86_64::_MM_HINT_T2);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = addr;
    }
}
#[inline(always)]
pub fn prefetch_l1(addr: *const u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_prefetch(addr as *const i8, core::arch::x86_64::_MM_HINT_T0);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = addr;
    }
}
#[inline(always)]
pub fn clflushopt(addr: *const u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_clflush(addr as *mut u8);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = addr;
    }
}
pub struct DdioPerfCounters {
    pub poll_cycles: u64,
    pub poll_count: u64,
    pub busy_spins: u64,
}
impl Default for DdioPerfCounters {
    fn default() -> Self {
        Self {
            poll_cycles: 0,
            poll_count: 0,
            busy_spins: 0,
        }
    }
}
