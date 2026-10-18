pub const VFIO_GET_API_VERSION: u64 = 15200;
pub const VFIO_CHECK_EXTENSION: u64 = 15201;
pub const VFIO_SET_IOMMU: u64 = 15202;
pub const VFIO_GROUP_GET_STATUS: u64 = 15203;
pub const VFIO_GROUP_SET_CONTAINER: u64 = 15204;
pub const VFIO_GROUP_GET_DEVICE_FD: u64 = 15206;
pub const VFIO_DEVICE_GET_INFO: u64 = 15207;
pub const VFIO_DEVICE_GET_REGION_INFO: u64 = 15208;
pub const VFIO_IOMMU_MAP_DMA: u64 = 15213;
pub const VFIO_IOMMU_UNMAP_DMA: u64 = 15214;
pub const VFIO_TYPE1_IOMMU: u64 = 1;
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct VfioGroupStatus {
    pub argsz: u32,
    pub flags: u32,
}
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct VfioDeviceInfo {
    pub argsz: u32,
    pub flags: u32,
    pub num_regions: u32,
    pub num_irqs: u32,
}
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct VfioRegionInfo {
    pub argsz: u32,
    pub flags: u32,
    pub index: u32,
    pub cap_offset: u32,
    pub size: u64,
    pub offset: u64,
}
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct VfioIommuDmaMap {
    pub argsz: u32,
    pub flags: u32,
    pub vaddr: u64,
    pub iova: u64,
    pub size: u64,
}
