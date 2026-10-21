//! CPU 翻译身份：ASID 及其失效。

mod asid;
mod error;
mod shootdown;

pub use asid::Asid;

pub(crate) use asid::vacant;
pub(crate) use shootdown::{deallocate, flush, occupy, shootdown, sync_instructions, vacate};
#[cfg(debug_assertions)]
pub(crate) use shootdown::{fail_next_shootdown, fail_shootdown, restore_shootdown};
