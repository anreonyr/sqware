#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate self as env;
extern crate self as execution;
extern crate self as resource;

pub use abi::{
    Access, AcquireSource, FlatReference, PAGE_SIZE, PieToken, Placement, ReferenceRange,
    ReleaseMode, TeamId, ViewControl,
};
pub use abi::{Frame, Mark, Permission, PieInfo, PieKind, PoleResult, TaskId, WireCodes, wire};

#[path = "fixtures/memory.rs"]
pub mod memory;
#[path = "fixtures/pie.rs"]
pub mod pie;
#[path = "fixtures/raw.rs"]
pub mod raw;
#[path = "fixtures/source.rs"]
pub mod source;
#[path = "fixtures/state.rs"]
mod state;

#[path = "../../src/driver/iommu/pages.rs"]
pub mod pages;
#[path = "../../src/driver/iommu/table.rs"]
pub mod table;

#[path = "fixtures/kernel.rs"]
mod kernel;
pub use kernel::Kernel;

pub mod dock {
    pub type Dock = resource_impl::mapping::Dock<crate::Kernel>;
    pub use resource_impl::view::View;
}
#[path = "fixtures/core.rs"]
mod core_fixture;
#[path = "fixtures/platform.rs"]
pub mod platform;
pub use iommu_api as api;
#[path = "../../src/driver/iommu/acquire.rs"]
pub mod acquire;
#[path = "../../src/driver/iommu/binding.rs"]
pub mod binding;
#[path = "../../src/driver/iommu/domain.rs"]
pub mod domain;
#[path = "../../src/driver/iommu/domain_id.rs"]
pub mod domain_id;
#[path = "../../src/driver/iommu/domain_range.rs"]
pub mod domain_range;
#[path = "../../src/driver/iommu/engine.rs"]
pub mod engine;
#[path = "../../src/driver/iommu/cache_step.rs"]
pub mod cache_step;
#[path = "../../src/driver/iommu/invalidation.rs"]
pub mod invalidation;
#[path = "../../src/driver/iommu/layout.rs"]
pub mod layout;
#[path = "../../src/driver/iommu/context.rs"]
pub mod context;
#[path = "../../src/driver/iommu/core.rs"]
pub mod iommu_core;
pub use iommu_hold as hold;
#[path = "../../src/driver/iommu/phase.rs"]
pub mod phase;
#[path = "../../src/driver/iommu/queue.rs"]
pub mod queue;
#[path = "../../src/driver/iommu/fence.rs"]
pub mod fence;
#[path = "../../src/driver/iommu/handshake.rs"]
pub mod handshake;
#[path = "../../src/driver/iommu/ticket.rs"]
pub mod ticket;
#[path = "../../src/driver/iommu/cookie.rs"]
pub mod cookie;
#[path = "../../src/driver/iommu/regs.rs"]
pub mod regs;
#[path = "../../src/driver/iommu/references.rs"]
pub mod references;
#[path = "../../src/driver/iommu/settled.rs"]
pub mod settled;

pub mod driver {
    pub mod iommu {
        pub use crate::{acquire, api, binding, context, cookie, domain, domain_id, domain_range,
            engine, fence, handshake, hold, invalidation, layout, iommu_core as core, pages, phase,
            queue, references, regs, settled, table, ticket};
        pub mod order { pub fn sync() {} }
    }
    pub mod shared {
        pub use crate::platform;
        pub mod dma {
            pub use hub_api::window_mark;
        }
    }
}

pub mod chrono { pub fn clock() -> u64 { crate::core_fixture::clock() } }
pub mod unit { pub fn self_id() -> crate::TaskId { crate::TaskId::new(5) } }
pub mod debug { pub fn put(_: &str) {} }
pub mod order { pub fn sync() {} }
pub mod pole { pub fn shut(_: crate::PieToken) -> abi::PoleResult<()> { Ok(()) } }

mod cleanup;
mod translation;
mod coordination;
mod contexts;
mod domains;
