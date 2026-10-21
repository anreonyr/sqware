//! Held references outlive CPU authority until the platform stops external access.
//!
//! 归**驱动自己的功能域**（`driver::iommu`）：只有这一台驱动持有它——它记的是
//! 「这一段帧被保留到硬件确认停止为止」这一件事，与供给来源的授权账是两回事。

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
