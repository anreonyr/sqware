#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate self as env;
extern crate self as resource;

pub use abi::{
    Frame, Mark, PAGE_SIZE, Permission, PieFail, PieInfo, PieKind, PieToken, PoleFail, TaskId, WireCodes, wire,
};
pub type PoleResult<T> = Result<T, PoleFail>;

#[path = "fixtures/state.rs"]
mod state;
#[path = "fixtures/raw.rs"]
pub mod raw;
#[path = "fixtures/pie.rs"]
pub mod pie;
#[path = "fixtures/pole.rs"]
pub mod pole;
#[path = "fixtures/kernel.rs"]
mod kernel;
pub use kernel::Kernel;

pub mod dock {
    pub type Dock = resource_impl::mapping::Dock<crate::Kernel>;
    pub use resource_impl::view::View;
}
#[path = "../../src/driver/pci/ecam.rs"]
pub mod ecam;
#[path = "../../src/driver/virtio/registers.rs"]
pub mod registers;
#[path = "../../src/driver/virtio/caps.rs"]
pub mod caps;
#[path = "../../src/driver/virtio/device.rs"]
pub mod virtio_device;
#[path = "../../src/driver/pci/api.rs"]
pub mod pci_api;
#[path = "../../src/driver/virtio/blk/recovery/api.rs"]
pub mod api;
#[path = "../../src/driver/virtio/blk/client/mod.rs"]
mod client;
#[path = "../../src/driver/shared/coords.rs"]
mod coords;
#[path = "../../src/harness/device/command.rs"]
mod command;
#[path = "../../src/driver/virtio/blk/recovery/attach_request.rs"]
pub mod attach_request;
#[path = "../../src/driver/virtio/blk/recovery/attachment.rs"]
pub mod attachment;
#[path = "../../src/driver/virtio/blk/recovery/executor.rs"]
pub mod executor;

pub mod debug {
    pub fn put(_: &str) {}
}

pub mod driver {
    pub mod pci {
        pub use crate::{ecam, pci_api as api};
    }
    pub mod virtio {
        pub use crate::caps::Caps;
        pub use crate::virtio_device::{
            notify_offset, queue_enable, reset, ring, status, Virtio,
        };

        pub mod device {
            pub use crate::virtio_device::{queue_enable, reset, reset_begin, status};
        }

        pub mod blk {
            pub mod recovery {
                pub use crate::{api, attach_request, attachment, executor};
            }
        }
    }
    pub mod shared {
        pub use hub_api::coord as dma;
        pub mod order {
            pub fn sync() {
                core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
            }
        }
    }
    pub mod iommu {
        pub mod order {
            pub fn sync() {
                core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

mod cleanup;
mod protocol;
