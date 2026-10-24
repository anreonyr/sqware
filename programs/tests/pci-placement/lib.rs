#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate self as env;
extern crate self as resource;
extern crate self as execution;

pub use abi::{
    AcquireSource, FlatReference, Frame, GrantRange, HoleFail, HoleLimits, HoleResult, Mark,
    MemoryFail, MemoryResult, PAGE_SIZE, Permission, PieFail, PieResult, PieToken, Placement,
    PoleFail, PoleResult, ReferenceRange, ReleaseMode, TaskId, TeamId, ViewControl, ViewId,
    WireCodes, make_fail, wire,
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
#[path = "fixtures/reference_state.rs"]
mod reference_state;
#[path = "fixtures/unseal.rs"]
mod unseal;
#[path = "fixtures/root.rs"]
mod root;
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
pub use pci_api as api;
#[path = "../../src/driver/pci/bar.rs"]
pub mod bar;
#[path = "../../src/driver/pci/endpoint.rs"]
pub mod endpoint;
#[path = "../../src/driver/pci/function_state.rs"]
pub mod function_state;
#[path = "../../src/driver/pci/place_request.rs"]
pub mod place_request;
#[path = "../../src/driver/pci/placement.rs"]
pub mod placement;
#[path = "../../src/driver/pci/reader.rs"]
pub mod reader;
#[path = "../../src/driver/pci/core.rs"]
pub mod pci_core;

pub mod driver {
    pub mod pci {
        pub use crate::{
            api, bar, ecam, endpoint, function_state, pci_core as core, place_request, placement,
            reader,
        };
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
    use crate::{FlatReference, PieToken, PoleResult};

    pub fn unseal(reference: FlatReference, shared: bool) -> PoleResult<PieToken> {
        crate::state::unseal(reference, shared)
    }

    pub fn shut(_: PieToken) -> PoleResult<()> {
        Ok(())
    }
}

#[path = "../../../crates/execution/src/memory/reference.rs"]
pub mod reference;

pub struct View;

impl View {
    pub fn new(_: crate::TeamId, _: crate::ViewId, _: usize, _: usize) -> Self {
        Self
    }
}

pub fn acquire(
    source: crate::AcquireSource,
    retention: crate::PieToken,
) -> crate::MemoryResult<crate::FlatReference> {
    crate::state::acquire(source, retention)
}

pub fn release(reference: crate::FlatReference) -> crate::MemoryResult<()> {
    crate::state::release_reference(reference)
}

pub fn attach(
    _: crate::ReferenceRange,
    _: crate::TeamId,
    _: crate::Placement,
    _: u64,
    _: crate::ViewControl,
) -> crate::MemoryResult<(crate::ViewId, usize)> {
    panic!("pci fixture: attach is not used")
}

pub mod memory {
    pub use crate::reference::Reference;
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
mod multibar;
