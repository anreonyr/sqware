#![no_std]
#![feature(allocator_api)]

extern crate alloc;

mod core;
pub mod entry;
mod machine;

use ::core::sync::atomic::{AtomicBool, Ordering};

static TESTING: AtomicBool = AtomicBool::new(false);

pub fn testing_mode() {
    TESTING.store(true, Ordering::Relaxed);
}

pub(crate) fn testing() -> bool {
    TESTING.load(Ordering::Relaxed)
}
