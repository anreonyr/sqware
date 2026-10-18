#![cfg(debug_assertions)]
use ::core::fmt;

#[macro_export]
macro_rules! expect {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            panic!("[health] {}", format_args!($($arg)*));
        }
    };
    ($cond:expr) => {
        $crate::expect!($cond, "expectation failed")
    };
}

#[cfg(debug_assertions)]
#[allow(dead_code)]
pub(crate) fn report_ok(item: &str, detail: fmt::Arguments) {
    crate::putln!("[health] {item}: ok ({detail})");
}

pub use crate::core::work::mail::hole::tests as hole;
pub use crate::core::work::room::conductor::tests::acceptance as scheduler;
pub use crate::entry::boot::error::accept as boot_errors;

pub mod backtrace;
pub mod boot_devices;
pub mod depend;
pub mod fault;
pub mod flat;
pub mod frame;
pub mod frame_layout;
#[cfg(feature = "fresh-scene")]
pub mod fresh_scene;
pub mod hart;
pub mod large_pages;
pub mod memory;
pub mod nest;
pub mod nest_failure;
pub mod pagetable;
pub mod permit;
pub mod pole;
#[cfg(feature = "race-scene")]
pub mod race_scene;
pub mod residency;
#[allow(dead_code)]
#[path = "../../../programs/src/system/supply/retention/mod.rs"]
pub(crate) mod retention_core;
pub mod shell;
pub mod source_views;
pub mod spare;
pub mod stress;
mod support;
pub mod syscall;
pub mod task;
pub mod transfer;

pub fn run() {
    #[cfg(debug_assertions)]
    {
        depend::accept();
        spare::accept();
        pagetable::pagetable();
        stress::accept();
        shell::accept();
        permit::form();
        permit::members();
        permit::fanout();
        permit::subs();
        permit::order();
        permit::badge();
        hart::count();
    }
}
