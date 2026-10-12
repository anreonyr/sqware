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
    pub identity_install: Option<system_api::identity::InstallId>,
}
pub struct Execution {
    pub instance: Option<Instance>,
    pub task: Option<TaskId>,
    pub deadline: u64,
}
pub struct Operation {
    pub key: OperationKey,
    pub progress: ExecutionProgress,
}
pub struct OperationKey {
    pub id: crate::system::assembly::JobId,
    pub request: Request,
    pub waiters: Vec<ReplySender<Said>>,
}
pub struct ExecutionProgress {
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
pub(crate) mod schedule;

impl Tracked {
    pub(crate) fn new(id: crate::system::assembly::JobId, request: Request) -> Self {
        Self {
            operation: Operation {
                key: OperationKey {
                    id,
                    request,
                    waiters: Vec::new(),
                },
                progress: ExecutionProgress {
                    execution: Execution {
                        instance: None,
                        task: None,
                        deadline: env::chrono::clock()
                            + crate::support::timing::BOOT_MS as u64 * 1_000_000,
                    },
                    failure: None,
                },
            },
            cursor: Cursor::default(),
            complete: false,
        }
    }

    pub(super) fn finish(
        &mut self,
        result: Result<::schedule::Progress, Fail>,
    ) -> Result<(), crate::system::app::Fault> {
        match result {
            Ok(::schedule::Progress::Done) => self.complete = true,
            Ok(::schedule::Progress::Pending) => {}
            Err(fail) => {
                let job = &mut self.operation;
                if matches!(job.key.request.action, Action::Ruin)
                    && job.progress.execution.task.is_some()
                {
                    programs::debug::put(&alloc::format!(
                        "system: ruin {} failed {:?}",
                        job.key.request.name,
                        fail
                    ));
                    return Err(crate::system::app::Fault::Shutdown);
                }
                job.progress.failure = Some(fail);
                if matches!(job.key.request.action, Action::Mint | Action::Embark { .. })
                    && job.progress.execution.instance.is_some()
                {
                    job.key.request.action = Action::Ruin;
                    job.progress.execution.deadline =
                        env::chrono::clock() + crate::support::timing::BOOT_MS as u64 * 1_000_000;
                    self.cursor.reset();
                } else {
                    self.complete = true;
                }
            }
        }
        Ok(())
    }
}

impl Active {
    pub(crate) fn task(&self) -> Option<TaskId> {
        self.0.as_ref().and_then(|job| job.progress.execution.task)
    }
}

mod startup;
pub(crate) use startup::{Startup, eligibility, startup};
