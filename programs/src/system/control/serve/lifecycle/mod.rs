use super::{start, unit::Service};
use crate::system::control::core::verdict::Fail;
use alloc::{collections::VecDeque, string::String, vec::Vec};
use env::{Mark, TaskId};
use ipc::rpc::reply::Sender as ReplySender;
use system_api::control::frame::Said;
use ::schedule::Cursor;

pub mod debark;
pub mod embark;
pub mod mint;
pub mod ruin;

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
pub struct Operations(pub VecDeque<Tracked>);
impl Operations {
    pub fn new() -> Self {
        Self(VecDeque::new())
    }
    pub fn push(&mut self, request: Request) -> Result<(), (Fail, Request)> {
        if self
            .0
            .iter()
            .any(|j| !j.complete && j.operation.request.name == request.name)
        {
            return Err((Fail::NotReady, request));
        }
        if self.0.try_reserve(1).is_err() {
            return Err((Fail::Full, request));
        }
        self.0.push_back(Tracked {
            operation: Operation {
                request,
                failure: None,
                execution: Execution {
                    instance: None,
                    task: None,
                    deadline: env::chrono::clock() + start::BOOT_MS as u64 * 1_000_000,
                },
            },
            cursor: Cursor::default(),
            complete: false,
        });
        Ok(())
    }
}
