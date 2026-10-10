#![no_std]

extern crate alloc;

pub mod contract;
pub mod field;
pub mod message;
pub use contract::Contract;
pub use field::{Field, Span, fetch_bytes, fetch_tail, store_bytes, store_tail, times, total};
pub use message::Message;

pub const OK: u8 = 0;
