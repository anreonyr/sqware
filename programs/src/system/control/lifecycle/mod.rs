use super::serve::unit::Service;
use crate::system::control::core::verdict::Fail;
use ::schedule::Cursor;
use alloc::{string::String, vec::Vec};
use env::{Mark, TaskId};
use ipc::rpc::reply::Sender as ReplySender;
use system_api::control::frame::Said;

mod debark;
mod embark;
mod mint;
mod ruin;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Mint,
    Embark,
    Debark,
    Ruin,
}
#[derive(Clone, Copy)]
pub enum Action {
    Mint,
    Embark { parent: Option<TaskId> },
    Debark,
    Ruin,
}
pub struct Request {
    pub name: String,
    pub action: Action,
    pub back: Option<ReplySender<Said>>,
}
pub struct Instance {
    pub service: Service,
    pub marks: Vec<Mark>,
    pub launched: bool,
}
pub struct Execution {
    pub instance: Option<Instance>,
    pub task: Option<TaskId>,
    pub deadline: u64,
}
pub struct Operation {
    pub request: Request,
    pub execution: Execution,
    pub failure: Option<Fail>,
}
pub struct Active(pub Option<Operation>);
pub struct Tracked {
    pub operation: Operation,
    pub cursor: Cursor,
    pub complete: bool,
}
mod dispatch;
mod queue;
pub(crate) mod schedule;
pub(crate) use queue::Operations;
