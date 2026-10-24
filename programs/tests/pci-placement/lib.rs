#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate self as env;
extern crate self as resource;

pub use abi::{
    Frame, GrantRange, HoleFail, HoleLimits, HoleResult, Mark, PAGE_SIZE, Permission, PieFail,
    PieResult, PieToken, PoleFail, PoleResult, ReleaseMode, TaskId, WireCodes, make_fail, wire,
};

pub mod debug {
    pub fn put(_: &str) {}
}

#[path = "../../../crates/resource/src/capability.rs"]
pub mod capability;
#[path = "../../../crates/resource/src/loan.rs"]
pub mod loan;

#[path = "fixtures/state.rs"]
mod state;
#[path = "fixtures/ecam.rs"]
pub mod ecam;
#[path = "fixtures/pie.rs"]
pub mod pie;
#[path = "fixtures/platform.rs"]
pub mod platform;
#[path = "fixtures/raw.rs"]
pub mod raw;
#[path = "../../../crates/resource/src/dock.rs"]
pub mod dock;

#[path = "../../src/driver/pci/api.rs"]
pub mod pci_api;
#[path = "../../src/driver/pci/endpoint.rs"]
pub mod endpoint;
#[path = "../../src/driver/pci/placement.rs"]
pub mod placement;
#[path = "../../src/driver/pci/virtio.rs"]
pub mod virtio;
#[path = "../../src/driver/pci/core.rs"]
pub mod pci_core;

pub mod driver {
    pub mod pci {
        pub use crate::{ecam, endpoint, pci_api as api, placement, virtio};
    }
    pub mod shared {
        pub use crate::platform;
    }
    pub mod iommu {
        pub mod order {
            pub fn sync() {}
        }
    }
}

pub mod room {
    pub fn starve() {}
}

pub mod chrono {
    pub fn clock() -> u64 {
        0
    }
}

pub mod pole {
    use crate::{PieToken, PoleResult};

    pub fn shut(_: PieToken) -> PoleResult<()> {
        Ok(())
    }
}

pub mod hole {
    use crate::{HoleLimits, HoleResult, Mark, PieToken};

    pub fn unseal(_: Mark, _: HoleLimits) -> HoleResult<PieToken> {
        Ok(PieToken::mint(900))
    }

    pub fn seal(_: PieToken) -> HoleResult<()> {
        Ok(())
    }
}

mod cleanup;
