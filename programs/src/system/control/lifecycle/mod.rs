use super::unit::Service;
use crate::system::control::unit::verdict::Fail;
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

impl Active {
    pub(crate) fn is_launching(
        &self,
        name: &str,
    ) -> Result<bool, crate::system::control::unit::verdict::Fail> {
        let job = self
            .0
            .as_ref()
            .ok_or(crate::system::control::unit::verdict::Fail::Unknown)?;
        Ok(job.request.name == name
            && job
                .execution
                .instance
                .as_ref()
                .is_some_and(|instance| !instance.launched))
    }
    pub(crate) fn is_named(&self, name: &str) -> bool {
        self.0.as_ref().is_some_and(|job| job.request.name == name)
    }
    pub(crate) fn task(&self) -> Option<TaskId> {
        self.0.as_ref().and_then(|job| job.execution.task)
    }
}

mod startup;
pub(crate) use startup::{Startup, eligibility, startup};
