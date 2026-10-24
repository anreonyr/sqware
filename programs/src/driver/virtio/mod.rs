//! Shared virtio-pci device protocol and the resident block unit, independent of the PCI backend.

pub mod blk;
pub mod caps;
pub mod device;
mod registers;

pub use caps::Caps;
pub use device::{notify_offset, queue_enable, reset, ring, status, Virtio};
pub use registers::NO_VECTOR;
