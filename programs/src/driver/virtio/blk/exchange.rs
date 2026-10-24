//! The one in-flight peer exchange owned by a binding operation.

use ipc::rpc::request::Pending;

use crate::driver::iommu::api as iommu;
use crate::driver::pci::api as pci;

pub enum Exchange {
    Pci(Pending<pci::Call>),
    Iommu(Pending<iommu::Call>),
}
