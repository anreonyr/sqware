//! Shared virtio-pci device protocol, independent of the PCI backend.

pub mod caps;
pub mod device;
mod registers;

pub use caps::Caps;
pub use device::{notify_offset, queue_enable, reset, ring, status, Virtio};
pub use registers::NO_VECTOR;
