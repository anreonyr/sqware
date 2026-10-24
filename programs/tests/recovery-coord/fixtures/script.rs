use alloc::vec::Vec;

use super::action::Action;

pub struct Script {
    pub actions: Vec<Action>,
    pub pci: Vec<crate::pci_api::Wire>,
    pub iommu: Vec<crate::iommu_api::Wire>,
    pub place_ok: bool,
    pub place_status: u8,
    pub enable_ok: bool,
    pub disable_ok: bool,
    pub pci_release_ok: bool,
    pub acquire_ok: bool,
    pub publish_ok: bool,
    pub invalidate_ok: bool,
    pub quiesce_ok: bool,
    pub iommu_release_ok: bool,
    pub iommu_status: u8,
    pub publish_pending: bool,
    pub iova: u64,
    pub attach_ok: bool,
    pub stop_ok: bool,
    pub detach_ok: bool,
}

impl Script {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
            pci: Vec::new(),
            iommu: Vec::new(),
            place_ok: true,
            place_status: 0,
            enable_ok: true,
            disable_ok: true,
            pci_release_ok: true,
            acquire_ok: true,
            publish_ok: true,
            invalidate_ok: true,
            quiesce_ok: true,
            iommu_release_ok: true,
            iommu_status: 0,
            publish_pending: false,
            iova: 0x2_0000_0000,
            attach_ok: true,
            stop_ok: true,
            detach_ok: true,
        }
    }

    pub fn action(&mut self, action: Action) {
        self.actions.push(action);
    }
}
