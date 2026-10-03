use env::{Mark, Permission, PieFail, PieToken, ProgramKind, TaskId, UnitFail, Wait};
use runtime::env::mail;
use runtime::env::room;
use runtime::env::unit as utask;

use crate::system::control::core::unit::{Announce, Service, Slot, State, Table};
use crate::system::control::core::verdict::{Fail, Ready, Reaped, admit_start, probe_ready};
use protocol::communication::session::establish::Endpoint;

use crate::unit::{Died, hub::E_HUB};

fn unit_fail(e: erra::Error<UnitFail>) -> Fail {
    match e.source {
        UnitFail::BadImage => Fail::BadImage,
        UnitFail::OoM => Fail::Full,
        UnitFail::Denied | UnitFail::Busy | UnitFail::BadEntry => Fail::Unknown,
    }
}

fn pie_fail(e: erra::Error<PieFail>) -> Fail {
    match e.source {
        PieFail::OoM => Fail::Full,
        PieFail::Denied
        | PieFail::Dead
        | PieFail::HandedOver
        | PieFail::NotAligned
        | PieFail::Busy => Fail::Unknown,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Tree(Died),
    Room(Died),
    Desk(Died),
    Book(Died),
    Face(Died),
    Load(Died),
    Dead(Died),
}

impl Start {
    pub fn code(self) -> env::Reason {
        match self {
            Start::Tree(d)
            | Start::Room(d)
            | Start::Desk(d)
            | Start::Book(d)
            | Start::Face(d)
            | Start::Load(d)
            | Start::Dead(d) => d,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Start::Tree(E_HUB) => "hub: tree",
            Start::Load(E_HUB) => "hub: no machine",
            Start::Face(E_HUB) => "hub: no league plate",
            Start::Room(E_HUB) => "hub: no room",
            Start::Desk(E_HUB) => "hub: desk",
            Start::Dead(E_HUB) => "inner: group dead",
            _ => "start: ?",
        }
    }
}

impl crate::Exit for Start {
    fn report(&self) -> crate::Report<'_> {
        crate::Report::note(self.code(), self.text())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    pub token: PieToken,
    pub perm: Permission,
}

pub fn mint(
    table: &mut Table,
    name: &str,
    image: &[u8],
    kind: ProgramKind,
) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let image = runtime::core::loader::build(image, kind).map_err(unit_fail)?;
    let team = image.team();
    let Ok(task) = image.spawn(&[], 0) else {
        return Err(Fail::Full);
    };
    if let Err(fail) = table.attach(name, Some(team), task) {
        let _ = room::doom(task);
        let _ = utask::oust(team);
        return Err(fail);
    }
    table.set_state(name, State::Starting);
    protocol::debug::put(&alloc::format!("system: minted {name} tid={}", task.get()));
    Ok(task)
}

pub fn start(
    table: &mut Table,
    name: &str,
    task: TaskId,
    grants: &[Grant],
    channels: &mut [Endpoint],
    marks: &[Mark],
    millis: Wait,
) -> Result<(), Fail> {
    let launched = (|| -> Result<(), Fail> {
        for g in grants {
            mail::accord(g.token, task, g.perm, Mark::NONE).map_err(pie_fail)?;
        }
        utask::hatch(task).map_err(unit_fail)
    })();
    if let Err(e) = launched {
        let _ = room::doom(task);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, name, channels, marks, millis)?;
    Ok(())
}

pub fn ready(
    table: &mut Table,
    name: &str,
    channels: &mut [Endpoint],
    marks: &[Mark],
    millis: Wait,
) -> Result<bool, Fail> {
    if let Ready::Up = probe_ready(table, name) {
        table.set_state(name, State::Ready);
        return Ok(true);
    }
    let Some(Service {
        slot: Slot::Live { task, .. },
        announce,
        ..
    }) = table.find(name)
    else {
        return Err(Fail::NotReady);
    };
    let (task, announce) = (*task, *announce);

    if announce == Announce::None {
        if !utask::join(task, Wait::POLL).unwrap_or(true) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }

    if !marks.is_empty()
        && channels.len() == marks.len()
        && channels
            .iter_mut()
            .zip(marks)
            .all(|(channel, mark)| channel.claim(task, *mark, millis))
    {
        table.set_state(name, State::Ready);
        return Ok(false);
    }
    if utask::join(task, Wait::POLL).unwrap_or(true) {
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    if millis == Wait::POLL {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

pub fn stop(table: &mut Table, name: &str) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { task, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let task = *task;
    let _ = room::doom(task);
    table.set_state(name, State::Stopping);
    Ok(())
}

pub fn until(table: &Table, name: &str, millis: Wait) -> Result<Reaped, Fail> {
    let Some(task) = live_task(table, name) else {
        return Err(Fail::Unknown);
    };
    if utask::join(task, Wait::POLL).unwrap_or(true) {
        return Ok(Reaped::Now);
    }
    if millis == Wait::POLL {
        return Ok(Reaped::Unsettled);
    }
    let _ = utask::join(task, millis);
    if utask::join(task, Wait::POLL).unwrap_or(true) {
        Ok(Reaped::Unsettled)
    } else {
        Ok(Reaped::Waited)
    }
}

fn live_task(table: &Table, name: &str) -> Option<TaskId> {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { task, .. },
            ..
        }) => Some(*task),
        _ => None,
    }
}

pub fn watch(table: &mut Table, name: &str, millis: Wait) -> Result<bool, Fail> {
    match until(table, name, millis)? {
        Reaped::Now | Reaped::Waited => {
            table.set_state(name, State::Dead);
            Ok(true)
        }
        Reaped::Unsettled => Ok(false),
    }
}
