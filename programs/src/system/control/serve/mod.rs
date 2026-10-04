pub mod answer;
pub mod living;
pub mod material;
pub mod publication;
pub mod reap;
pub mod resource;
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

pub(crate) mod driver;
pub mod frame;
pub mod lifecycle;

pub mod instance;
pub mod run;
pub mod schedule;

pub mod install;
