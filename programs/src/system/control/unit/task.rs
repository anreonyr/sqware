use env::room;
use env::unit;
use env::{Mark, Permission, PieFail, PieToken, ProgramKind, TaskId, UnitFail, Wait};

use crate::system::control::unit::table::{Announce, Service, Slot, State, Table};
use crate::system::control::unit::verdict::{Fail, Ready, Reaped, admit_mint, probe_ready};
use ipc::session::Endpoint;

use env::pie;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    pub token: PieToken,
    pub perm: Permission,
}

pub struct Image<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub kind: ProgramKind,
}
pub struct Readiness<'a> {
    pub name: &'a str,
    pub marks: &'a [Mark],
    pub wait: Wait,
}
pub struct Launch<'a> {
    pub task: TaskId,
    pub grants: &'a [Grant],
    pub readiness: Readiness<'a>,
}

pub(crate) fn mint(table: &mut Table, name: &str, built: system_api::loader::Built) -> Result<TaskId, Fail> {
    let system_api::loader::Built { task, team } = built;
    if let Err(fail) = admit_mint(table, name) {
        let _ = room::doom(task);
        let _ = unit::oust(team);
        return Err(fail);
    }
    if let Err(fail) = table.attach(
        name,
        Slot::Live {
            team: Some(team),
            task,
        },
    ) {
        let _ = room::doom(task);
        let _ = unit::oust(team);
        return Err(fail);
    }
    table.set_state(name, State::Starting);
    programs::debug::put(&alloc::format!("system: minted {name} tid={}", task.get()));
    Ok(task)
}

pub fn embark(
    table: &mut Table,
    launch: Launch<'_>,
    channels: &mut [Endpoint],
) -> Result<(), Fail> {
    let Launch {
        task,
        grants,
        readiness,
    } = launch;
    let name = readiness.name;
    let launched = (|| -> Result<(), Fail> {
        for g in grants {
            pie::accord(g.token, task, g.perm, Mark::NONE).map_err(pie_fail)?;
        }
        unit::embark(task).map_err(unit_fail)
    })();
    if let Err(e) = launched {
        let _ = room::doom(task);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, readiness, channels)?;
    Ok(())
}

pub fn ready(
    table: &mut Table,
    readiness: Readiness<'_>,
    channels: &mut [Endpoint],
) -> Result<bool, Fail> {
    let Readiness {
        name,
        marks,
        wait: millis,
    } = readiness;
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
        if !unit::join(task, Wait::POLL).unwrap_or(true) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }

    if !marks.is_empty() && channels.len() == marks.len() {
        let mut claimed = true;
        for (channel, mark) in channels.iter_mut().zip(marks) {
            match channel.claim(task, *mark, millis) {
                Ok(true) => {}
                Ok(false) => {
                    claimed = false;
                    break;
                }
                Err(ipc::session::establish::DiscoveryFail::Ambiguous) => {
                    return Err(Fail::Unknown);
                }
                Err(ipc::session::establish::DiscoveryFail::Missing) => return Err(Fail::NotReady),
            }
        }
        if claimed {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
    }
    if unit::join(task, Wait::POLL).unwrap_or(true) {
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    if millis == Wait::POLL {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

pub fn ruin(table: &mut Table, name: &str) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { task, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let task = *task;
    unit::slay(task).map_err(unit_fail)?;
    table.set_state(name, State::Stopping);
    Ok(())
}

pub fn until(table: &Table, name: &str, millis: Wait) -> Result<Reaped, Fail> {
    let Some(task) = live_task(table, name) else {
        return Err(Fail::Unknown);
    };
    Ok(super::wait::until(task, millis))
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
