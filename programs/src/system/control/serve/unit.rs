use super::start::{Error, READY_MS, RETRY_MS};
use super::task as service;
use crate::service::hub::bridge::Activation;
use crate::system::control::core::unit::{Slot, State, Table};
use crate::system::control::core::verdict::{self as core, Fail};
use crate::system::identity::serve::install::Roster;
use crate::system::life::{Phase, Status};
use crate::unit::UnitFile;
use ::core::time::Duration;
use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use env::{TaskId, Wait};
use protocol::communication::session::establish::Endpoint;

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
            .register(
                name,
                if program.supply().is_empty() {
                    crate::system::control::core::unit::Announce::None
                } else {
                    crate::system::control::core::unit::Announce::Channel
                },
                restart,
            )
            .map_err(|_| Error::Table)
    }
    pub fn state(&self, name: String) -> Result<State, Fail> {
        if matches!(name.as_str(), "operator" | "identity") {
            let task = self.task(&name).ok_or(Fail::Unknown)?;
            if runtime::env::unit::join(task, Wait::POLL).unwrap_or(true) {
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
    pub fn ruin(
        &mut self,
        name: String,
        roster: &Roster,
        activation: &mut Option<Activation>,
    ) -> Result<(), Fail> {
        if matches!(name.as_str(), "operator" | "identity") {
            return Err(Fail::Unknown);
        }
        let task = self.task(name.as_str()).ok_or(Fail::Unknown)?;
        service::ruin(&mut self.table, name.as_str())?;
        if name == "hub" {
            *activation = None;
        }
        roster.unbind(task).map_err(|_| Fail::NotReady)
    }
    pub fn debark(&mut self, name: String) -> Result<(), Fail> {
        if matches!(name.as_str(), "operator" | "identity") { return Err(Fail::Unknown); }
        let task = self.task(&name).ok_or(Fail::Unknown)?;
        if self.table.find(&name).is_none_or(|r| r.state != State::Ready) { return Err(Fail::NotReady); }
        let until = runtime::env::chrono::clock() + READY_MS as u64 * 1_000_000;
        loop {
            match runtime::env::unit::debark(task) {
                Ok(()) => { self.table.set_state(&name, State::Debarked); return Ok(()); }
                Err(e) if matches!(e.source, env::UnitFail::Busy) && runtime::env::chrono::clock() < until => {
                    runtime::env::room::sleep(Duration::from_millis(RETRY_MS as u64)).map_err(|_| Fail::NotReady)?;
                }
                Err(_) => return Err(Fail::NotReady),
            }
        }
    }
    pub(super) fn resume(&mut self, name: &str) -> Result<Service, Fail> {
        let task = self.task(name).ok_or(Fail::Unknown)?;
        if self.table.find(name).is_none_or(|r| r.state != State::Debarked) { return Err(Fail::NotReady); }
        runtime::env::unit::embark(task).map_err(|_| Fail::NotReady)?;
        self.table.set_state(name, State::Ready);
        Ok((task, Vec::new()))
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
            Slot::Live { task, .. } if matches!(row.state, State::Starting | State::Ready | State::Debarked) => {
                Some(task)
            }
            _ => None,
        })
    }
    pub(crate) fn discard(
        &mut self,
        name: &str,
        task: TaskId,
        roster: &Roster,
        activation: &mut Option<Activation>,
    ) {
        if name == "hub" {
            *activation = None;
        }
        let _ = runtime::env::room::doom(task);
        if let Some(row) = self.table.find(name) {
            if let Slot::Live {
                team: Some(team), ..
            } = row.slot
            {
                let _ = runtime::env::unit::oust(team);
            }
        }
        if let Err(why) = roster.unbind(task) {
            protocol::debug::put(&alloc::format!("system: compensation {name}: {why}"));
        }
        self.table.detach(name);
        self.table.set_state(name, State::Dead);
    }
    pub fn due(&self) -> bool {
        core::due(&self.table)
    }
    pub fn done(&self) -> bool {
        core::done(&self.table)
    }
    pub fn await_ready(&self, name: &str, wait: Wait) -> Result<(), Fail> {
        if matches!(name, "operator" | "identity") {
            return if self
                .status
                .phase
                .load(::core::sync::atomic::Ordering::Acquire)
                == Phase::Running as u8
            {
                Ok(())
            } else {
                Err(Fail::NotReady)
            };
        }
        let mut left = match wait {
            Wait::POLL => 0,
            Wait::AtMost(ms) => ms,
            Wait::Forever => READY_MS,
        };
        loop {
            match self.table.find(name).map(|s| s.state) {
                Some(State::Ready | State::Stopping | State::Dead) => return Ok(()),
                Some(State::NeverStarted | State::Starting | State::Debarked) => {}
                None => return Err(Fail::Unknown),
            }
            if left == 0 {
                return Err(Fail::NotReady);
            }
            let _ = runtime::env::room::sleep(Duration::from_millis(RETRY_MS as u64));
            left -= 1;
        }
    }
    pub fn ruin_rest(&mut self, roster: &Roster, activation: &mut Option<Activation>) {
        let names: Vec<String> = self
            .table
            .living()
            .filter(|row| {
                matches!(row.state, State::Starting | State::Ready | State::Debarked)
                    && !matches!(row.slot, Slot::Live { team: None, .. })
            })
            .map(|row| row.name.clone())
            .collect();
        for name in &names {
            let _ = self.ruin(name.clone(), roster, activation);
        }
    }
}
