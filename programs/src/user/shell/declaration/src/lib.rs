#![no_std]
//! Shell-owned host-safe declarations shared by the Shell runtime and composition.

pub use unit;

pub mod commands;
pub mod program;

pub use commands::{BINDINGS, Command, COMMANDS};
pub use program::PROGRAM;
