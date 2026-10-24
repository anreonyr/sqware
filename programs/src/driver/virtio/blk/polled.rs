use crate::driver::{iommu::api as iommu, pci::api as pci};

pub(super) enum Polled {
    Pci(Result<Option<pci::Said>, ()>),
    Iommu(Result<Option<iommu::Said>, ()>),
    Empty,
}
