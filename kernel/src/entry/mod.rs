pub mod boot;
pub(crate) mod call;
pub(crate) mod report;
pub(crate) mod trap;

#[cfg(debug_assertions)]
#[path = "../../tests/health/mod.rs"]
pub mod check;
