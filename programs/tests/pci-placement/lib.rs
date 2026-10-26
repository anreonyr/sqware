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

#[path = "fixtures/kernel.rs"]
mod kernel;
pub use kernel::Kernel;

pub mod capability {
    pub type Capability = resource_impl::capability::Capability<crate::Kernel>;
}

pub mod loan {
    pub type Loan<'a> = resource_impl::loan::Loan<'a, crate::Kernel>;
}

pub mod dock {
    pub type Dock = resource_impl::mapping::Dock<crate::Kernel>;
    pub use resource_impl::view::View;
}

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

#[path = "fixtures/reference_kernel.rs"]
mod reference_kernel;
pub use reference_kernel::FixtureKernel;

pub mod reference {
    pub type Reference =
        execution_impl::memory::mechanism::Reference<crate::FixtureKernel>;
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
