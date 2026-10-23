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

mod reclamation;
mod service;
pub use service::Service;
mod wait;
pub(crate) mod guard;
pub struct Control {
    pub(in crate::system::control) reclaimed_at: u64,
    pub(in crate::system::control) status: Arc<Status>,
    pub(in crate::system::control) targets: &'static [crate::unit::Target],
    pub(in crate::system::control) table: Table,
    pub(in crate::system::control) inputs: Vec<start::Input>,
    pub(in crate::system::control) internal: Vec<TaskId>,
    pub(in crate::system::control) instances:
        Vec<crate::system::control::instance::state::Instance>,
    pub(in crate::system::control) pending: Vec<Pending>,
    pub(in crate::system::control) guarded: &'static [&'static str],
    pub(in crate::system::control) guard_token: Option<env::PieToken>,
}
pub(crate) struct Pending {
    pub(crate) name: &'static str,
    pub(crate) service: Service,
}
impl Control {
    pub fn new(status: Arc<Status>, targets: &'static [crate::unit::Target]) -> Self {
        Self {
            reclaimed_at: 0,
            status,
            targets,
            table: Table::new(),
            inputs: Vec::new(),
            internal: Vec::new(),
            pending: Vec::new(),
            instances: Vec::new(),
            guarded: &[],
            guard_token: None,
        }
    }
    pub(crate) fn guard_required(&self, name: &str) -> bool {
        guard::required(self.guarded, name)
    }
    pub(crate) fn guard_held(&self, name: &str) -> bool {
        self.table.guard_held(name)
    }
    pub(crate) fn hold_guard(&mut self, name: &str) {
        self.table.set_guard_held(name, true);
    }
    pub(crate) fn clear_guard(&mut self, name: &str) {
        self.table.set_guard_held(name, false);
    }
    pub fn enlist(&mut self, program: &UnitFile) -> Result<(), Error> {
        if !program.valid() {
            return Err(Error::Step("invalid supply declaration"));
        }
        let name = program.name().to_string();
        let restart = program.control.ending.ok_or(Error::Step("no ending"))?;
        self.table
            .register(Declaration {
                name,
                announce: match program.control.ready {
                    crate::unit::Ready::OnSpawn => {
                        crate::system::control::unit::table::Announce::None
                    }
                    crate::unit::Ready::Signal(_) => {
                        crate::system::control::unit::table::Announce::Channel
                    }
                },
                restart,
            })
            .map_err(|_| Error::Table)
    }
    pub fn state(&self, name: String) -> Result<State, Fail> {
        if let Some(bootstrap) = bootstrap_named(&name) {
            let task = self.bootstrap_task(bootstrap).ok_or(Fail::Unknown)?;
            if env::unit::join_task(task, Wait::POLL).unwrap_or(true) {
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
        let row = self.table.find(name).ok_or(Fail::Unknown)?;
        if row.state != State::Debarked {
            return Err(Fail::NotReady);
        }
        let Slot::Live {
            team: Some(team), ..
        } = row.slot
        else {
            return Err(Fail::Unknown);
        };
        env::unit::embark_team(team).map_err(|_| Fail::NotReady)?;
        self.table.set_state(name, State::Ready);
        Ok(())
    }
    pub(crate) fn task(&self, name: &str) -> Option<TaskId> {
        match self.table.find(name)?.slot {
            Slot::Live { task, .. } => Some(task),
            Slot::None => None,
        }
    }
    pub(crate) fn task_team(&self, task: TaskId) -> Option<env::TeamId> {
        self.table.rows().find_map(|row| match row.slot {
            Slot::Live { task: live, team } if live == task => team,
            _ => None,
        })
    }
    pub(crate) fn bootstrap_task(&self, task: crate::unit::Bootstrap) -> Option<TaskId> {
        use ::core::sync::atomic::Ordering;
        let raw = match task {
            crate::unit::Bootstrap::Operator => self.status.operator.load(Ordering::Acquire),
            crate::unit::Bootstrap::Identity => self.status.identity.load(Ordering::Acquire),
        };
        (raw != 0).then(|| TaskId::new(raw))
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
            && !env::unit::join_task(task, Wait::POLL).unwrap_or(true)
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
                [
                    self.bootstrap_task(crate::unit::Bootstrap::Operator),
                    self.bootstrap_task(crate::unit::Bootstrap::Identity),
                ]
                .into_iter()
                .flatten(),
            )
            .chain(
                self.instances
                    .iter()
                    .filter(|item| {
                        item.team().is_some()
                            && matches!(
                                item.state,
                                State::Starting | State::Ready | State::Debarked
                            )
                    })
                    .map(|item| item.task),
            )
    }
}

fn bootstrap_named(name: &str) -> Option<crate::unit::Bootstrap> {
    match name {
        "operator" => Some(crate::unit::Bootstrap::Operator),
        "identity" => Some(crate::unit::Bootstrap::Identity),
        _ => None,
    }
}

mod fixture;
mod intent;
mod observe;
mod publisher;
pub(crate) mod reap;
pub mod start;
pub mod task;

pub mod table;
pub(crate) mod verdict;
