use super::{start, unit::Service};
use crate::system::control::core::verdict::Fail;
use ::schedule::Cursor;
use alloc::{collections::VecDeque, string::String, vec::Vec};
use env::{Mark, TaskId};
use ipc::rpc::reply::Sender as ReplySender;
use system_api::control::frame::Said;

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
pub struct Operations(pub(in crate::system::control) VecDeque<Tracked>);
impl Operations {
    pub fn new() -> Self {
        Self(VecDeque::new())
    }
    pub(in crate::system::control) fn push(
        &mut self,
        request: Request,
    ) -> Result<(), (Fail, Request)> {
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
    pub(crate) fn submit(&mut self, name: String, action: Action) -> Result<(), Fail> {
        self.push(Request {
            name,
            action,
            back: None,
        })
        .map_err(|(fail, _)| fail)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|job| job.operation.request.name == name)
    }
    pub(crate) fn completed_local_action(&self) -> Option<Result<Action, Fail>> {
        let job = self
            .0
            .iter()
            .find(|job| job.complete && job.operation.request.back.is_none())?;
        Some(match job.operation.failure {
            Some(fail) => Err(fail),
            None => Ok(job.operation.request.action),
        })
    }
    pub(crate) fn take_local_completion(
        &mut self,
        name: &str,
    ) -> Option<Result<Option<TaskId>, Fail>> {
        let at = self.0.iter().position(|job| {
            job.complete
                && job.operation.request.back.is_none()
                && job.operation.request.name == name
        })?;
        let job = self.0.remove(at)?;
        Some(match job.operation.failure {
            Some(fail) => Err(fail),
            None => Ok(job.operation.execution.task),
        })
    }
    pub(crate) fn reply_completed(&mut self) {
        let count = self.0.len();
        for _ in 0..count {
            if let Some(mut tracked) = self.0.pop_front() {
                if tracked.complete && tracked.operation.request.back.is_some() {
                    super::answer::complete(&mut tracked.operation);
                } else {
                    self.0.push_back(tracked);
                }
            }
        }
    }
    pub(crate) fn retire_local_completed(&mut self) {
        self.0
            .retain(|job| !job.complete || job.operation.request.back.is_some());
    }
}
