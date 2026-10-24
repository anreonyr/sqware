#![cfg(test)]
#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

pub use abi::{Access, Frame, Mark, PieToken, ReleaseMode, TaskId, Wait, WireCodes, wire};

#[path = "../../src/driver/pci/api.rs"]
pub mod pci_api;
#[path = "../../src/driver/iommu/api.rs"]
pub mod iommu_api;
#[path = "../../src/driver/recovery/api.rs"]
pub mod recovery_api;
#[path = "../../src/driver/recovery/attach_request.rs"]
pub mod attach_request;
#[path = "../../src/driver/shared/coords.rs"]
pub mod coords;
#[path = "../../src/driver/recovery/operation.rs"]
pub mod operation;
#[path = "../../src/driver/recovery/profile.rs"]
pub mod profile;
#[path = "../../src/driver/recovery/stage.rs"]
pub mod stage;
#[path = "../../src/driver/recovery/coord.rs"]
pub mod coord;

pub mod debug { pub fn put(_: &str) {} }
pub mod unit { pub fn self_id() -> crate::TaskId { crate::TaskId::new(1) } }
pub mod pie {
    pub fn release(_: crate::PieToken, _: crate::ReleaseMode) -> Result<(), ()> {
        crate::fixtures::handoff::release()
    }
}
mod fixtures;
pub mod raw { pub use crate::fixtures::handoff::Hole; }
pub mod session {
    pub mod establish {
        pub fn give(_: env::TaskId, _: env::Mark) -> Result<env::PieToken, ()> {
            crate::fixtures::handoff::give()
        }
    }
}
pub mod driver {
    pub mod pci { pub use crate::pci_api as api; }
    pub mod iommu { pub use crate::iommu_api as api; }
    pub mod shared {
        pub mod dma { pub use crate::coords::Coords; pub use hub_api::coord_mark; }
        pub use crate::fixtures::peer;
    }
    pub mod recovery {
        pub use crate::recovery_api as api;
        pub use crate::{coord, operation, profile, stage};
        pub use crate::fixtures::core;
    }
}
mod tests;
