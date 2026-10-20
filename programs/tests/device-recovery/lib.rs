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
#[path = "../../../crates/resource/src/dock.rs"]
pub mod dock;
#[path = "../../src/driver/pci/ecam.rs"]
pub mod ecam;
#[path = "../../src/driver/pci/virtio.rs"]
pub mod virtio;
#[path = "../../src/driver/pci/api.rs"]
pub mod pci_api;
#[path = "../../src/driver/recovery/api.rs"]
pub mod recovery_api;
#[path = "../../src/driver/recovery/attach_request.rs"]
pub mod attach_request;
#[path = "../../src/driver/recovery/device.rs"]
pub mod device;
#[path = "../../src/driver/recovery/core.rs"]
pub mod recovery_core;

pub mod debug {
    pub fn put(_: &str) {}
}

pub mod driver {
    pub mod pci {
        pub use crate::{ecam, pci_api as api, virtio};
    }
    pub mod recovery {
        pub use crate::{attach_request, device, recovery_api as api};
    }
    pub mod shared {
        pub use hub_api::coord as dma;
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
