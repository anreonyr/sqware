use self::start::Error;
use crate::system::app::life::{Phase, Status};
use crate::system::control::unit::table::{Declaration, Slot, State, Table};
use crate::system::control::unit::verdict::Fail;
use crate::unit::UnitFile;
use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use env::{TaskId, Wait};

mod service;
pub use service::Service;
mod wait;
pub struct Control {
    pub(in crate::system::control) status: Arc<Status>,
    pub(in crate::system::control) table: Table,
    pub(in crate::system::control) inputs: Vec<start::Input>,
    pub(in crate::system::control) internal: Vec<TaskId>,
    pub(in crate::system::control) instances:
        Vec<crate::system::control::instance::state::Instance>,
    pub(in crate::system::control) pending: Vec<Pending>,
}
pub(crate) struct Pending {
    pub(crate) name: &'static str,
    pub(crate) service: Service,
}
impl Control {
    pub fn new(status: Arc<Status>) -> Self {
        Self {
            status,
            table: Table::new(),
            inputs: Vec::new(),
            internal: Vec::new(),
            pending: Vec::new(),
            instances: Vec::new(),
        }
    }
    pub fn enlist(&mut self, program: &UnitFile) -> Result<(), Error> {
        if !program.valid() {
            return Err(Error::Step("invalid supply declaration"));
        }
        let name = program.name().to_string();
        let restart = program.relation.restart.ok_or(Error::Step("no ending"))?;
        self.table
            .register(Declaration {
                name,
                announce: if program.supply().is_empty() {
                    crate::system::control::unit::table::Announce::None
                } else {
                    crate::system::control::unit::table::Announce::Channel
                },
                restart,
            })
            .map_err(|_| Error::Table)
    }
    pub fn state(&self, name: String) -> Result<State, Fail> {
        if matches!(name.as_str(), "operator" | "identity") {
            let task = self.task(&name).ok_or(Fail::Unknown)?;
            if env::unit::join(task, Wait::POLL).unwrap_or(true) {
                return Ok(State::Dead);
            }
            return Ok(
                match self
                    .status
                    .phase
                    .load(::core::sync::atomic::Ordering::Acquire)
                {
                    phase if phase == Phase::Starting as u8 => State::Starting,
                    phase if phase == Phase::Running as u8 => State::Ready,
                    _ => State::Stopping,
                },
            );
        }
        self.table
            .find(name.as_str())
            .map(|s| s.state)
            .ok_or(Fail::Unknown)
    }

    pub(in crate::system::control) fn resume(&mut self, name: &str) -> Result<(), Fail> {
        let task = self.task(name).ok_or(Fail::Unknown)?;
        if self
            .table
            .find(name)
            .is_none_or(|r| r.state != State::Debarked)
        {
            return Err(Fail::NotReady);
        }
        env::unit::embark(task).map_err(|_| Fail::NotReady)?;
        self.table.set_state(name, State::Ready);
        Ok(())
    }
    pub(crate) fn task(&self, name: &str) -> Option<TaskId> {
        use ::core::sync::atomic::Ordering;
        match name {
            "operator" => return Some(TaskId::new(self.status.operator.load(Ordering::Acquire))),
            "identity" => return Some(TaskId::new(self.status.identity.load(Ordering::Acquire))),
            _ => {}
        }
        match self.table.find(name)?.slot {
            Slot::Live { task, .. } => Some(task),
            Slot::None => None,
        }
    }
    pub(crate) fn register_internal(&mut self, task: TaskId) -> Result<(), &'static str> {
        self.internal
            .try_reserve(1)
            .map_err(|_| "internal lifecycle capacity")?;
        self.internal.push(task);
        Ok(())
    }
    pub(crate) fn live(&self, task: TaskId) -> bool {
        (task == env::unit::self_id() || self.tasks().any(|known| known == task))
            && !env::unit::join(task, Wait::POLL).unwrap_or(true)
    }
    pub(crate) fn internal_tasks(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.internal.iter().copied()
    }
    pub(crate) fn tasks(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.table
            .living()
            .filter_map(|row| match row.slot {
                Slot::Live { task, .. }
                    if matches!(row.state, State::Starting | State::Ready | State::Debarked) =>
                {
                    Some(task)
                }
                _ => None,
            })
            .chain(self.internal_tasks())
            .chain(
                self.instances
                    .iter()
                    .filter(|item| {
                        item.team.is_some()
                            && matches!(
                                item.state,
                                State::Starting | State::Ready | State::Debarked
                            )
                    })
                    .map(|item| item.task),
            )
    }
}

mod fixture;
mod observe;
pub(crate) mod reap;
pub mod start;
pub mod task;

pub mod table;
pub(crate) mod verdict;
