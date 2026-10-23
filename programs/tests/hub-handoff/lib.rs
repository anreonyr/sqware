#![allow(dead_code)]
extern crate alloc;
extern crate env;
extern crate hub_api;
extern crate system_api;

extern crate self as ipc;
extern crate self as resource;

pub mod backend;
pub mod raw;
pub mod rpc;

#[path = "../../src/service/hub/core/mod.rs"]
pub mod core;

pub mod service {
    pub mod hub {
        pub use crate::core;
    }
}

pub mod serve;
mod enrollment;

pub mod debug {
    pub fn put(_: &str) {}
}

#[cfg(test)]
mod cases;
