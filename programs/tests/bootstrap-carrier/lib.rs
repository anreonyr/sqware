#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as resource;

pub use abi::{HoleLimits, Mark, Permission, PieKind, PieToken, ReleaseMode, TaskId, Wait};

mod effects;
pub use effects::Effects;
mod pie_info;
pub use pie_info::PieInfo;

use std::cell::RefCell;
thread_local! {
    pub static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default());
}

pub mod hole;
pub mod pie;
pub mod port;
pub mod raw;
pub mod system;
pub mod unit;

pub mod chrono {
    pub fn clock() -> u64 {
        crate::EFFECTS.with(|effects| effects.borrow().now)
    }
}
#[path = "../../src/support/timing.rs"]
pub mod timing;
pub mod support { pub use crate::timing; }

#[path = "../../src/system/client/src/bootstrap.rs"]
pub mod client;

#[cfg(test)]
mod tests;
