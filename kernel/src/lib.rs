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
mod resource;
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

pub fn init(dtp: usize) -> Result<(), boot::BootError> {
    console::init();
    platform::machine::init(dtp).map_err(|source| boot::BootError::Machine { dtp, source })?;
    allocator::init().map_err(boot::BootError::Allocator)?;
    memory::manager::mode::detect().map_err(boot::BootError::PagingMode)?;
    unit::init().map_err(boot::BootError::KernelSpace)?;
    clock::init().map_err(boot::BootError::Clock)?;
    trace::init().map_err(boot::BootError::Trace)?;
    trap::resources::init().map_err(|source| boot::BootError::Resources {
        operation: boot::ResourceOperation::InitializeTraps,
        source,
    })?;
    runtime::switcher::envcall::resources::init().map_err(|source| boot::BootError::Resources {
        operation: boot::ResourceOperation::InitializeCalls,
        source,
    })?;
    trap::init().map_err(boot::BootError::TrapStacks)?;
    Ok(())
}
