//! Operator owns the namespace, guest capabilities and subscriptions.
#[derive(Debug)]
pub enum Fail {
    Tree,
    Desk,
    Room,
    Dead,
}
mod answer;
mod door;
mod frame;
pub(crate) mod plate;
pub mod run;
mod schedule;
mod session;
mod tip;
mod watch;
