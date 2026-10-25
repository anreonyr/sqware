use super::unit::Service;
use crate::system::control::unit::verdict::Fail;
use ::schedule::Cursor;
use alloc::{string::String, vec::Vec};
use env::TaskId;
use ipc::rpc::reply::Sender as ReplySender;
use system_api::control::frame::Said;

mod debark;
mod embark;
mod mint;
mod ruin;

pub(crate) mod authority;
mod authority_verify;
mod directive;
mod intent;
mod origin;
pub(crate) mod takeover;
mod transition;

pub use directive::Directive;
pub use origin::Origin;
pub(crate) use intent::accepted as accept_intent;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Mint,
    Embark,
    Debark,
    Ruin,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Mint,
    Recover,
    Embark { parent: Option<TaskId> },
    Debark,
    Ruin,
}
pub struct Request {
    pub name: String,
    pub directive: Directive,
    pub back: Option<ReplySender<Said>>,
}
pub struct Instance {
    pub service: Service,
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
    pub program: Option<&'static crate::unit::UnitFile>,
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
mod finish;
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
                program: None,
            },
            cursor: Cursor::default(),
            complete: false,
        }
    }
}

impl Active {
    pub(crate) fn task(&self) -> Option<TaskId> {
        self.0.as_ref().and_then(|job| job.progress.execution.task)
    }
}

mod startup;
pub(crate) use startup::{Startup, eligibility, startup};
