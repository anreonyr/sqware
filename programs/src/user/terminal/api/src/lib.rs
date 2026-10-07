#![no_std]

pub mod frame;
pub mod marks;
pub const REGISTRY: &[&[env::marks::Definition]] = &[marks::DECLARATIONS];
const _: () = assert!(env::marks::conflict(REGISTRY).is_none());
