//! References outlive CPU authority until the trusted stop authority confirms external access ended.

mod close;
mod domain_id;
mod domain_range;
mod effects;
mod exposure;
mod failure;
mod holds;
mod memory;
mod occupancy;
mod phase;
mod platform;
mod range;
mod registration;
mod retention_id;
mod scope;
mod scope_id;

pub use close::Close;
pub use domain_id::DomainId;
pub use domain_range::DomainRange;
pub use effects::MemoryEffects;
pub use failure::Failure;
pub use holds::Holds;
pub use occupancy::Occupancy;
pub use phase::Phase;
pub use platform::Platform;
pub use range::AddressRange;
pub use registration::Registration;
pub use retention_id::RetentionId;
pub use scope::Scope;
pub use scope_id::ScopeId;
