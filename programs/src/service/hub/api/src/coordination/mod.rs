//! Generic coordination frames between Hub and an independently scheduled
//! hardware driver. The driver owns concrete bus/IOMMU work; this face carries
//! only the Hub-qualified binding identity and the execution it serves.

pub mod call;
pub mod prepare;
pub mod release;
pub mod wire;

pub use call::Call;
pub use prepare::Prepare;
pub use release::Release;
pub use wire::Wire;

pub const PREPARE: u8 = 1;
pub const RELEASE: u8 = 2;
