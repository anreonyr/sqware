pub mod core;
pub(crate) mod identity;
pub(crate) mod instance;
pub(crate) mod lifecycle;
pub mod serve;
pub mod unit;

#[derive(Debug)]
pub enum Fail {
    Room,
    Publication,
    Dead,
    Wait,
    Idle,
    Shutdown,
}
