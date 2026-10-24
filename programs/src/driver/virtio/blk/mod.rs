//! VirtIO block device Driver: protocol discovery, binding coordination and reset.
//! PCI and IOMMU effects remain behind authenticated provider endpoints.
//! Hub owns claims and binding identity; Control supplies the stopped-target proof.

pub mod client;
pub mod serve;
pub(crate) mod coord;
pub(crate) mod exchange;
pub(crate) mod operation;
pub(crate) mod peers;
pub(crate) mod polled;
pub(crate) mod profile;
pub(crate) mod recovery;
pub(crate) mod stage;
