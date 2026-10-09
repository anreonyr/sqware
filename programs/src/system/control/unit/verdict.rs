use crate::unit::Ending;

use super::table::{Announce, Slot, State, Table};

pub fn admit_mint(table: &Table, name: &str) -> Result<(), Fail> {
    let Some(s) = table.find(name) else {
        return Err(Fail::Unknown);
    };
    match s.state {
        State::NeverStarted | State::Dead => Ok(()),
        State::Starting | State::Ready | State::Stopping | State::Debarked => Err(Fail::NotReady),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    Up,
    Gone,
    Pending,
}

pub fn probe_ready(table: &Table, name: &str) -> Ready {
    let Some(s) = table.find(name) else {
        return Ready::Gone;
    };
    match s.state {
        State::Ready => Ready::Up,
        State::Starting => match (s.announce, s.slot) {
            (Announce::None, Slot::Live { .. }) => Ready::Up,
            (Announce::Channel, Slot::Live { .. }) => Ready::Pending,
            (_, Slot::None) => Ready::Gone,
        },
        State::Stopping | State::Debarked => Ready::Gone,
        State::NeverStarted | State::Dead => Ready::Gone,
    }
}

pub fn due(table: &Table) -> bool {
    table.living().all(|r| match r.restart {
        Ending::Resident => true,
        Ending::Transient | Ending::Told => false,
    })
}

pub fn walking(table: &Table) -> bool {
    table.living().any(|r| match r.restart {
        Ending::Transient => true,
        Ending::Resident | Ending::Told => false,
    })
}

pub fn done(table: &Table) -> bool {
    table.living().next().is_none()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reaped {
    Now,
    Waited,
    Unsettled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    Unknown,
    BadImage,
    Full,
    NotReady,
}
