//! Shared authenticated endpoints used by every physical device coordinator.

use env::TaskId;

use crate::driver::iommu::api as iommu;
use crate::driver::pci::api as pci;
use crate::driver::shared::peer::Peer;

pub struct Peers {
    pub hub: TaskId,
    pub pci: Peer<pci::Call>,
    pub iommu: Peer<iommu::Call>,
}
