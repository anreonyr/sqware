use super::{Action, Execution, Operation, Request, Tracked};
use crate::system::control::{unit::start, unit::verdict::Fail};
use ::schedule::{Cursor, Progress};
use alloc::{collections::VecDeque, string::String};
use env::TaskId;

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
                    super::super::service::answer::complete(&mut tracked.operation);
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

impl Tracked {
    pub(super) fn finish(
        &mut self,
        result: Result<Progress, Fail>,
    ) -> Result<(), crate::system::app::Fault> {
        match result {
            Ok(Progress::Done) => self.complete = true,
            Ok(Progress::Pending) => {}
            Err(fail) => {
                let job = &mut self.operation;
                if matches!(job.request.action, Action::Ruin) && job.execution.task.is_some() {
                    programs::debug::put(&alloc::format!(
                        "system: ruin {} failed {:?}",
                        job.request.name,
                        fail
                    ));
                    return Err(crate::system::app::Fault::Shutdown);
                }
                job.failure = Some(fail);
                if matches!(job.request.action, Action::Mint | Action::Embark { .. })
                    && job.execution.instance.is_some()
                {
                    job.request.action = Action::Ruin;
                    job.execution.deadline =
                        env::chrono::clock() + start::BOOT_MS as u64 * 1_000_000;
                    self.cursor.reset();
                } else {
                    self.complete = true;
                }
            }
        }
        Ok(())
    }
}
