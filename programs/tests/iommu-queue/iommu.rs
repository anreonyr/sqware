//! The production mechanism under test, plus the two shims it reaches for.
//!
//! `queue.rs` asks `super` for three things: `pages::Pages` (the retained window it
//! writes commands into and reads completions from), `regs` (the register layer) and
//! `order::sync` (the fence between its stores and the device's view). `regs` is real
//! here; the other two are shims, for the reasons in their own files.

#[path = "../../src/driver/iommu/queue.rs"]
pub mod queue;

#[path = "../../src/driver/iommu/regs.rs"]
pub mod regs;

pub mod order;
pub mod pages;
