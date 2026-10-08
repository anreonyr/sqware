pub(crate) mod boot;
pub(crate) mod bootstrap;
mod execute;
pub(crate) mod install;
pub(crate) mod life;
pub(crate) mod policy;
pub(crate) mod scene;
pub(crate) mod schedule;
pub(crate) mod wait;
pub use execute::run;

mod config;

mod waiting;

#[derive(Debug)]
pub(crate) enum Fault {
    Room,
    Publication,
    Dead,
    Wait,
    Idle,
    Shutdown,
}
