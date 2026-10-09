#![no_std]

extern crate alloc;

mod debug;
mod terminal;

pub use terminal::{Connection, Foreground, Io, Read, Terminal};
