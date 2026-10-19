use std::ptr;
const MAP_HUGE_2MB: libc::c_int = 21 << 26;
const MAP_HUGE_1GB: libc::c_int = 30 << 26;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HugePageKind {
    Huge1GB,
    Huge2MB,
    Regular,
}
impl std::fmt::Display for HugePageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HugePageKind::Huge1GB => write!(f, "1GB"),
            HugePageKind::Huge2MB => write!(f, "2MB"),
            HugePageKind::Regular => write!(f, "4KB"),
        }
    }
}
pub struct HugeAllocation<T> {
    pub ptr: *mut T,
    pub count: usize,
    pub kind: HugePageKind,
    pub byte_size: usize,
}
unsafe impl<T: Send> Send for HugeAllocation<T> {}
unsafe impl<T: Sync> Sync for HugeAllocation<T> {}
impl<T> HugeAllocation<T> {
    pub fn alloc(count: usize) -> anyhow::Result<Self> {
        let byte_size = count * std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>().max(4096);
        let (ptr, kind) = Self::try_1gb(byte_size)
            .or_else(|| Self::try_2mb(byte_size))
            .or_else(|| Self::try_regular(byte_size, align))
            .ok_or_else(|| anyhow::anyhow!("all allocation strategies exhausted"))?;
        Ok(Self {
            ptr: ptr as *mut T,
            count,
            kind,
            byte_size,
        })
    }
    fn try_1gb(byte_size: usize) -> Option<(*mut libc::c_void, HugePageKind)> {
        // SAFETY: Attempting 1GB hugepage mmap with MAP_HUGETLB | MAP_HUGE_1GB.
        unsafe {
            let ptr = libc::mmap(
                ptr::null_mut(),
                byte_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_HUGETLB | MAP_HUGE_1GB,
                -1,
                0,
            );
            if ptr != libc::MAP_FAILED {
                let _ = libc::mlock(ptr, byte_size);
                Some((ptr, HugePageKind::Huge1GB))
            } else {
                None
            }
        }
    }
    fn try_2mb(byte_size: usize) -> Option<(*mut libc::c_void, HugePageKind)> {
        // SAFETY: Attempting 2MB hugepage mmap with MAP_HUGETLB | MAP_HUGE_2MB.
        unsafe {
            let ptr = libc::mmap(
                ptr::null_mut(),
                byte_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_HUGETLB | MAP_HUGE_2MB,
                -1,
                0,
            );
            if ptr != libc::MAP_FAILED {
                let _ = libc::mlock(ptr, byte_size);
                Some((ptr, HugePageKind::Huge2MB))
            } else {
                None
            }
        }
    }
    fn try_regular(byte_size: usize, align: usize) -> Option<(*mut libc::c_void, HugePageKind)> {
        // SAFETY: Falling back to posix_memalign for standard 4KB page allocation.
        unsafe {
            let mut aligned_ptr: *mut libc::c_void = ptr::null_mut();
            let ret = libc::posix_memalign(&mut aligned_ptr, align, byte_size);
            if ret == 0 && !aligned_ptr.is_null() {
                let _ = libc::mlock(aligned_ptr, byte_size);
                Some((aligned_ptr, HugePageKind::Regular))
            } else {
                None
            }
        }
    }
}
impl<T> Drop for HugeAllocation<T> {
    fn drop(&mut self) {
        // SAFETY: Deallocating memory region via munmap (hugepage) or free (regular).
        unsafe {
            match self.kind {
                HugePageKind::Huge1GB | HugePageKind::Huge2MB => {
                    libc::munmap(self.ptr as *mut libc::c_void, self.byte_size);
                }
                HugePageKind::Regular => {
                    libc::free(self.ptr as *mut libc::c_void);
                }
            }
        }
    }
}

