use alloc::{string::String, vec::Vec};
use env::{TaskId, TeamId};

use crate::unit::Ending;

use super::verdict::Fail;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    NeverStarted,
    Starting,
    Ready,
    Stopping,
    Dead,
    Debarked,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    None,
    Live { team: Option<TeamId>, task: TaskId },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Announce {
    Channel,
    None,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Service {
    pub name: String,
    pub slot: Slot,
    pub state: State,
    pub announce: Announce,
    pub restart: Ending,
    pub named: bool,
    pub guard_held: bool,
    pub run_requested: bool,
}

const EMPTY: Service = Service {
    name: String::new(),
    slot: Slot::None,
    state: State::NeverStarted,
    announce: Announce::None,
    restart: Ending::Resident,
    named: false,
    guard_held: false,
    run_requested: false,
};

pub struct Table {
    rows: Vec<Service>,
}

impl Table {
    pub const CAP: usize = env::ledger::manifest::MAX_PROGRAMS;

    pub const fn new() -> Table {
        Table { rows: Vec::new() }
    }

    pub fn register(&mut self, declaration: Declaration) -> Result<(), Fail> {
        let Declaration {
            name,
            announce,
            restart,
        } = declaration;
        if self.find(name.as_str()).is_some() {
            return Err(Fail::Unknown);
        }
        if self.rows.len() >= Self::CAP {
            return Err(Fail::Full);
        }
        self.rows.try_reserve(1).map_err(|_| Fail::Full)?;
        self.rows.push(Service {
            name,
            announce,
            restart,
            ..EMPTY
        });
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&Service> {
        self.rows.iter().find(|s| s.name == name)
    }

    pub fn rows(&self) -> impl Iterator<Item = &Service> {
        self.rows.iter().filter(|s| !s.name.is_empty())
    }

    pub fn living(&self) -> impl Iterator<Item = &Service> {
        self.rows().filter(|s| !matches!(s.state, State::Dead))
    }

    pub fn set_state(&mut self, name: &str, state: State) {
        if let Some(s) = self.row_mut(name) {
            s.state = state;
        }
    }

    pub fn attach(&mut self, name: &str, slot: Slot) -> Result<(), Fail> {
        let Some(s) = self.row_mut(name) else {
            return Err(Fail::Unknown);
        };
        if s.guard_held {
            return Err(Fail::NotReady);
        }
        s.slot = slot;
        s.state = State::NeverStarted;
        s.named = false;
        Ok(())
    }

    pub fn detach(&mut self, name: &str) -> Result<(), Fail> {
        let Some(s) = self.row_mut(name) else {
            return Ok(());
        };
        if s.guard_held {
            return Err(Fail::NotReady);
        }
        s.slot = Slot::None;
        Ok(())
    }

    pub fn guard_held(&self, name: &str) -> bool {
        self.find(name).is_some_and(|row| row.guard_held)
    }

    pub fn set_guard_held(&mut self, name: &str, held: bool) {
        if let Some(s) = self.row_mut(name) {
            s.guard_held = held;
        }
    }

    pub fn set_run_requested(&mut self, name: &str, requested: bool) -> Result<(), Fail> {
        let Some(s) = self.row_mut(name) else {
            return Err(Fail::Unknown);
        };
        s.run_requested = requested;
        Ok(())
    }

    pub(crate) fn mark_static(&mut self, task: TaskId) {
        if let Some(row) = self
            .rows
            .iter_mut()
            .find(|r| matches!(r.slot, Slot::Live { task: at, .. } if at == task))
        {
            row.named = true;
        }
    }

    fn row_mut(&mut self, name: &str) -> Option<&mut Service> {
        self.rows.iter_mut().find(|s| s.name == name)
    }
}

pub struct Declaration {
    pub name: String,
    pub announce: Announce,
    pub restart: Ending,
}
