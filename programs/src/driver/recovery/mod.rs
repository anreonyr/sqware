//! Independent device Driver: protocol discovery, binding coordination and reset.
//! PCI and IOMMU effects remain behind authenticated provider endpoints.
//! Hub owns claims and binding identity; Control supplies the stopped-target proof.

pub mod api;
pub mod client;
pub mod coord;
pub mod core;
pub mod device;
pub mod operation;
pub mod profile;
pub mod serve;
pub mod stage;
mod attach_request;
