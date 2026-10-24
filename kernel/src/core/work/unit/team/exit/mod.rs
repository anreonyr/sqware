mod backend;
mod mode;
mod policy;
#[cfg(feature = "critical-scene")]
mod probe;
mod state;

pub(crate) use policy::ExitPolicy;

use env::{PieToken, TaskId, UnitFail};

use crate::core::work::mail::Mail;
use crate::core::work::pend;
use crate::core::work::unit::task::Task;

use super::life::TeamLife;

pub(crate) fn conflicts(life: &TeamLife) -> bool {
    let mut node = life.parent.as_deref();
    while let Some(team) = node {
        if team.exit.protected() {
            return true;
        }
        node = team.parent.as_deref();
    }
    let mut descendant = false;
    life.visit(&mut |team| {
        if team.id != life.id && team.exit.protected() {
            descendant = true;
        }
    });
    descendant
}

pub(crate) fn authorize(caller: &Task, token: PieToken) -> bool {
    let Some(pie) = pend::locate(caller, token) else {
        return false;
    };
    if !pie.alive()
        || !pie.permission().contains(env::Permission::FETCH)
        || pie.owner() != Some(caller.ident.id)
    {
        return false;
    }
    matches!(pie.mail(), Mail::Nole(meta) if meta.guards())
}

pub(crate) fn member_dying(task: &Task, reason: usize) {
    let mut node = Some(&*task.ident.team.life);
    while let Some(team) = node {
        if team.exit.trip() {
            backend::stop(reason);
        }
        node = team.parent.as_deref();
    }
}

pub(crate) fn arm(team: &TeamLife, owner: TaskId) -> Result<(), UnitFail> {
    team.exit.arm(owner)
}

pub(crate) fn disarm(team: &TeamLife, owner: TaskId) -> Result<(), UnitFail> {
    team.exit.disarm(owner)
}

#[cfg(feature = "critical-scene")]
pub(crate) fn cleanup_started(task: &Task) {
    crate::putln!("[critical] reference cleanup task={}", task.ident.id.get());
}
