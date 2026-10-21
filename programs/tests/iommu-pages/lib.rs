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

#[path = "fixtures/memory.rs"]
pub mod memory;
#[path = "fixtures/pie.rs"]
pub mod pie;
#[path = "fixtures/raw.rs"]
pub mod raw;
#[path = "fixtures/state.rs"]
mod state;

#[path = "../../src/driver/iommu/pages.rs"]
mod pages;
#[path = "../../src/driver/iommu/table.rs"]
mod table;

mod cleanup;
mod translation;
