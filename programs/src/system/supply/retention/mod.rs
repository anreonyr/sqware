//! Supply-owned references outlive CPU authority until the platform stops external access.

mod close;
mod effects;
mod failure;
mod holds;
mod memory;
mod phase;
mod platform;
mod range;
mod registration;
mod scope;

pub use close::Close;
pub use effects::MemoryEffects;
pub use failure::Failure;
pub use holds::Holds;
pub use phase::Phase;
pub use platform::Platform;
pub use range::DeviceRange;
pub use registration::Registration;
pub use scope::Scope;
