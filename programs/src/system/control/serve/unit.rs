use super::start::Error;
use crate::system::control::core::unit::{Declaration, Slot, State, Table};
use crate::system::control::core::verdict::Fail;
use crate::system::life::{Phase, Status};
use crate::unit::UnitFile;
use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use env::{TaskId, Wait};
use protocol::communication::session::Endpoint;

pub type Service = (TaskId, Vec<Endpoint>);
pub struct Control {
    pub(crate) status: Arc<Status>,
    pub(crate) table: Table,
    pub(crate) pending: Vec<Pending>,
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
            pending: Vec::new(),
        }
    }
    pub fn enlist(&mut self, program: &UnitFile) -> Result<(), Error> {
        let name = program.name().to_string();
        let restart = program.relation.restart.ok_or(Error::Step("no ending"))?;
        self.table
            .register(Declaration {
                name,
                announce: if program.supply().is_empty() {
                    crate::system::control::core::unit::Announce::None
                } else {
                    crate::system::control::core::unit::Announce::Channel
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

    pub(super) fn resume(&mut self, name: &str) -> Result<(), Fail> {
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
    pub(crate) fn tasks(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.table.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. }
                if matches!(row.state, State::Starting | State::Ready | State::Debarked) =>
            {
                Some(task)
            }
            _ => None,
        })
    }
}
