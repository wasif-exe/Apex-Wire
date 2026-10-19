use std::fs::File;
use std::os::unix::io::{AsRawFd, RawFd};
use std::ptr;
use crate::vfio_sys::*;
pub struct VfioDevice {
    pub container_fd: RawFd,
    pub group_fd: RawFd,
    pub device_fd: RawFd,
    pub bar0_ptr: *mut u8,
    pub bar0_size: usize,
    pub is_mock: bool,
}
unsafe impl Send for VfioDevice {}
unsafe impl Sync for VfioDevice {}
impl VfioDevice {
    pub fn open_mock(bar0_size: usize) -> anyhow::Result<Self> {
        let align = 64;
        let mut aligned_ptr: *mut libc::c_void = ptr::null_mut();
        // SAFETY: Allocating mock PCI BAR0 memory region for unit tests and benchmarks without physical NIC.
        unsafe {
            let ret = libc::posix_memalign(&mut aligned_ptr, align, bar0_size);
            if ret != 0 {
                return Err(anyhow::anyhow!("Mock BAR0 allocation failed"));
            }
            ptr::write_bytes(aligned_ptr as *mut u8, 0, bar0_size);
        }
        Ok(Self {
            container_fd: -1,
            group_fd: -1,
            device_fd: -1,
            bar0_ptr: aligned_ptr as *mut u8,
            bar0_size,
            is_mock: true,
        })
    }
    pub fn open_pci(group_id: u32, pci_addr: &str) -> anyhow::Result<Self> {
        let container_file = File::options().read(true).write(true).open("/dev/vfio/vfio")?;
        let container_fd = container_file.as_raw_fd();
        std::mem::forget(container_file);
        // SAFETY: Querying VFIO API version via ioctl.
        let version = unsafe { libc::ioctl(container_fd, VFIO_GET_API_VERSION as _, 0) };
        if version != 1 {
            // SAFETY: Closing container file descriptor on error.
            unsafe { libc::close(container_fd); }
            return Err(anyhow::anyhow!("Unsupported VFIO API version: {}", version));
        }
        let group_path = format!("/dev/vfio/{}", group_id);
        let group_file = match File::options().read(true).write(true).open(&group_path) {
            Ok(f) => f,
            Err(_) => {
                // SAFETY: Closing container file descriptor on error.
                unsafe { libc::close(container_fd); }
                return Err(anyhow::anyhow!("Failed to open VFIO group {}", group_path));
            }
        };
        let group_fd = group_file.as_raw_fd();
        std::mem::forget(group_file);
        let mut group_status = VfioGroupStatus {
            argsz: std::mem::size_of::<VfioGroupStatus>() as u32,
            flags: 0,
        };
        // SAFETY: Checking VFIO group status via ioctl.
        unsafe {
            if libc::ioctl(group_fd, VFIO_GROUP_GET_STATUS as _, &mut group_status as *mut _) < 0 {
                libc::close(group_fd);
                libc::close(container_fd);
                return Err(anyhow::anyhow!("VFIO_GROUP_GET_STATUS failed"));
            }
        }
        // SAFETY: Binding VFIO group to container.
        unsafe {
            if libc::ioctl(group_fd, VFIO_GROUP_SET_CONTAINER as _, &container_fd as *const _ as *const libc::c_void) < 0 {
                libc::close(group_fd);
                libc::close(container_fd);
                return Err(anyhow::anyhow!("VFIO_GROUP_SET_CONTAINER failed"));
            }
        }
        // SAFETY: Setting IOMMU type.
        unsafe {
            if libc::ioctl(container_fd, VFIO_SET_IOMMU as _, VFIO_TYPE1_IOMMU) < 0 {
                libc::close(group_fd);
                libc::close(container_fd);
                return Err(anyhow::anyhow!("VFIO_SET_IOMMU failed"));
            }
        }
        let c_pci_addr = std::ffi::CString::new(pci_addr)?;
        // SAFETY: Acquiring device file descriptor from group.
        let device_fd = unsafe { libc::ioctl(group_fd, VFIO_GROUP_GET_DEVICE_FD as _, c_pci_addr.as_ptr()) };
        if device_fd < 0 {
            // SAFETY: Cleaning group and container file descriptors.
            unsafe {
                libc::close(group_fd);
                libc::close(container_fd);
            }
            return Err(anyhow::anyhow!("VFIO_GROUP_GET_DEVICE_FD failed for {}", pci_addr));
        }
        let mut reg_info = VfioRegionInfo {
            argsz: std::mem::size_of::<VfioRegionInfo>() as u32,
            flags: 0,
            index: 0,
            cap_offset: 0,
            size: 0,
            offset: 0,
        };
        // SAFETY: Querying BAR0 region parameters.
        unsafe {
            if libc::ioctl(device_fd, VFIO_DEVICE_GET_REGION_INFO as _, &mut reg_info as *mut _) < 0 {
                libc::close(device_fd);
                libc::close(group_fd);
                libc::close(container_fd);
                return Err(anyhow::anyhow!("VFIO_DEVICE_GET_REGION_INFO failed for BAR0"));
            }
        }
        let bar0_size = reg_info.size as usize;
        // SAFETY: Memory mapping physical PCI BAR0 directly into process virtual memory address space.
        let bar0_ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                bar0_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                device_fd,
                reg_info.offset as libc::off_t,
            )
        };
        if bar0_ptr == libc::MAP_FAILED {
            // SAFETY: Cleaning file descriptors on mmap failure.
            unsafe {
                libc::close(device_fd);
                libc::close(group_fd);
                libc::close(container_fd);
            }
            return Err(anyhow::anyhow!("BAR0 mmap failed"));
        }
        Ok(Self {
            container_fd,
            group_fd,
            device_fd,
            bar0_ptr: bar0_ptr as *mut u8,
            bar0_size,
            is_mock: false,
        })
    }
    pub fn map_dma(&self, vaddr: *mut u8, iova: u64, size: usize) -> anyhow::Result<()> {
        if self.is_mock {
            return Ok(());
        }
        let map = VfioIommuDmaMap {
            argsz: std::mem::size_of::<VfioIommuDmaMap>() as u32,
            flags: 1 | 2,
            vaddr: vaddr as u64,
            iova,
            size: size as u64,
        };
        // SAFETY: Binding Virtual Address to Physical IOVA in IOMMU mapping table.
        unsafe {
            if libc::ioctl(self.container_fd, VFIO_IOMMU_MAP_DMA as _, &map as *const _) < 0 {
                return Err(anyhow::anyhow!("VFIO_IOMMU_MAP_DMA failed"));
            }
        }
        Ok(())
    }
}
impl Drop for VfioDevice {
    fn drop(&mut self) {
        // SAFETY: Unmapping memory regions and closing file descriptors on device drop.
        unsafe {
            if self.is_mock {
                if !self.bar0_ptr.is_null() {
                    libc::free(self.bar0_ptr as *mut libc::c_void);
                }
            } else {
                if !self.bar0_ptr.is_null() && self.bar0_size > 0 {
                    libc::munmap(self.bar0_ptr as *mut libc::c_void, self.bar0_size);
                }
                if self.device_fd >= 0 { libc::close(self.device_fd); }
                if self.group_fd >= 0 { libc::close(self.group_fd); }
                if self.container_fd >= 0 { libc::close(self.container_fd); }
            }
        }
    }
}

