//! The production mechanism under test, plus the shims it reaches for.
//!
//! `queue.rs` asks `super` for three things: `pages::Pages` (the retained window it
//! writes commands into and reads completions from), `regs` (the register layer) and
//! `order::sync` (the fence between its stores and the device's view). It also asks its
//! own layer for the three pieces of protocol state that live beside it: `handshake`
//! (the preserved deadline), `fence` (one `IOFENCE.C` step) and `outstanding` (the
//! in-flight entry). Those five are real production files; `order` and `pages` are
//! shims, for the reasons in their own files. `regs` is real here too.

#[path = "../../src/driver/iommu/fence.rs"]
pub mod fence;

#[path = "../../src/driver/iommu/handshake.rs"]
pub mod handshake;

#[path = "../../src/driver/iommu/outstanding.rs"]
pub mod outstanding;

#[path = "../../src/driver/iommu/queue.rs"]
pub mod queue;

#[path = "../../src/driver/iommu/regs.rs"]
pub mod regs;

pub mod order;
pub mod pages;
