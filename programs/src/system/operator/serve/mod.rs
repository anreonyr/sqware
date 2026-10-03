//! Operator owns the namespace, guest capabilities and subscriptions.
#[derive(Debug)]
pub enum Fail { Tree, Desk, Room, Dead }
mod answer;
mod door;
mod frame;
mod session;
mod tip;
mod schedule;
pub(crate) mod plate;
mod watch;
pub mod run;
pub mod install;
