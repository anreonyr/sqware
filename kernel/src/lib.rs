#![no_std]
#![feature(allocator_api)]

extern crate alloc;

pub mod boot;
mod console;
mod hart;
pub mod health;
mod layout;
mod lock;
mod memory;
mod platform;
mod runtime;
mod work;

use core::sync::atomic::{AtomicBool, Ordering};

use crate::memory::allocator;
use crate::runtime::chrono::clock;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::trap;
use crate::work::unit;

static TESTING: AtomicBool = AtomicBool::new(false);

pub fn testing_mode() {
    TESTING.store(true, Ordering::Relaxed);
}

pub(crate) fn testing() -> bool {
    TESTING.load(Ordering::Relaxed)
}

pub fn init(dtp: usize) {
    console::init();
    platform::machine::init(dtp);
    allocator::init().unwrap_or_else(|e| panic!("allocator init failed: {e}"));
    unit::init().unwrap_or_else(|e| panic!("unit init failed: {e}"));
    clock::init().unwrap_or_else(|e| panic!("clock init failed: {e}"));
    trace::init().unwrap_or_else(|e| panic!("trace init failed: {e}"));
    trap::init();
}

pub fn main(_hartid: usize, dtp: usize) -> ! {
    init(dtp);
    boot::banner();
    boot::init();
    boot::run()
}
