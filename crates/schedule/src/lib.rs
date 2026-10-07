#![no_std]

extern crate alloc;

#[path = "mod.rs"]
mod schedule;

pub use schedule::*;
