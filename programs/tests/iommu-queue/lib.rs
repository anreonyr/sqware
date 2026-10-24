//! The IOMMU command queue against a fake device.
//!
//! [`iommu`] compiles the **production** `driver/iommu/queue.rs` and
//! `driver/iommu/regs.rs` through `#[path]`, with shims for the two neighbours
//! production gets from its siblings (`pages`, `order`) and for the one environment
//! call it makes (`env::chrono::clock`). [`device`] is the other end of the register
//! window: it owns the pages and plays the hardware the queue talks to.
//!
//! What this fixture is **not**: there is no IOMMU, no PCIe, no second domain and no
//! real time. `tests.rs` records, per case, which part of the production path the
//! fixture can witness and which part it cannot.
//!
//! Run it with the host target spelled out:
//!
//! ```text
//! cargo test --manifest-path programs/tests/iommu-queue/Cargo.toml --target x86_64-unknown-linux-gnu
//! ```
//!
//! A bare `cargo test` from this directory walks up to the repository's
//! `.cargo/config.toml` and inherits the riscv64 target, which a host binary cannot
//! link (`no global allocator` / `no panic handler`).

extern crate alloc;
extern crate self as env;

pub mod device;
pub mod iommu;

pub mod chrono {
    /// The queue's only clock. Each read steps the fake time and lets the fake device
    /// run, so a bounded wait inside the real queue code ends on its own.
    pub fn clock() -> u64 {
        crate::device::clock()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod completion;
