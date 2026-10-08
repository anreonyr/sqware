pub mod answer;
pub(crate) mod create;
mod fixture;
mod instances;
pub mod material;
mod observe;
pub mod reap;
pub mod source;
pub mod start;
pub mod task;
pub mod unit;
pub mod watch;

#[derive(Debug)]
pub enum Fail {
    Room,
    Publication,
    Dead,
    Wait,
    Idle,
    Shutdown,
}

pub mod instance;

pub mod hook;
