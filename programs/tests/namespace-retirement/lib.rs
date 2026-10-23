#![allow(dead_code)]

extern crate alloc;
extern crate self as system_client;

pub mod operator {
    use env::PieToken;
    use system_api::operator::{Entry, GrantId};

    #[derive(Clone, Copy)]
    pub struct GrantInfo {
        pub id: GrantId,
        pub entry: Entry,
        pub entrance: PieToken,
    }
}

pub mod debug {
    pub fn put(_: &str) {}
}

pub mod fixtures;

#[path = "../../src/system/app/namespace/installed.rs"]
pub mod installed;
pub use installed::Installed;

pub use fixtures::system::System;

#[path = "../../src/system/app/namespace/retire.rs"]
mod retire;

pub mod system {
    pub mod app {
        pub mod namespace {
            pub use crate::{Installed, System};
        }
    }
}

#[cfg(test)]
mod cases;
