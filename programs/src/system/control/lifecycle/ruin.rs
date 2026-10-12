use super::super::unit::Control;
use super::Active;
use crate::system::{
    control::identity::Roster,
    control::unit::{
        table::{Slot, State},
        verdict::Fail,
    },
};
use ::schedule::{Progress, Res, ResMut};
use env::Wait;

pub(crate) fn pre(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    control.input(&job.key.request.name)?.program;
    let Some(task) = job
        .progress
        .execution
        .task
        .or_else(|| control.task(&job.key.request.name))
    else {
        if control
            .table
            .find(&job.key.request.name)
            .is_some_and(|r| r.state == State::Dead)
        {
            return Ok(Progress::Done);
        }
        return Err(Fail::Unknown);
    };
    job.progress.execution.task = Some(task);
    control
        .table
        .set_state(&job.key.request.name, State::Stopping);
    Ok(Progress::Done)
}
pub fn run(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.progress.execution.task {
        let team = match control
            .table
            .find(&job.key.request.name)
            .map(|row| row.slot)
        {
            Some(Slot::Live { team, .. }) => team,
            _ => None,
        };
        if let Some(team) = team {
            let _ = env::unit::slay_team(team);
        }
        if !matches!(env::unit::join_task(task, Wait::POLL), Ok(true)) {
            if team.is_none() {
                let _ = env::room::doom(task);
            }
            if !matches!(env::unit::join_task(task, Wait::POLL), Ok(true)) {
                return if env::chrono::clock() < job.progress.execution.deadline {
                    Ok(Progress::Pending)
                } else {
                    Err(Fail::NotReady)
                };
            }
        }
    }
    Ok(Progress::Done)
}
pub fn post(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(row) = control.table.find(&job.key.request.name) {
        if let Slot::Live {
            team: Some(team), ..
        } = row.slot
        {
            match env::unit::oust(team) {
                Ok(()) => {}
                Err(e)
                    if matches!(e.source, env::UnitFail::Busy)
                        && env::chrono::clock() < job.progress.execution.deadline =>
                {
                    return Ok(Progress::Pending);
                }
                Err(e) if matches!(e.source, env::UnitFail::Denied) => {}
                Err(_) => return Err(Fail::NotReady),
            }
        }
    }
    control.pending.retain(|p| p.name != job.key.request.name);
    Ok(Progress::Done)
}

pub fn release_supply(
    active: Res<Active>,
    mut supplies: ResMut<crate::system::app::supplies::Supplies>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.progress.execution.task {
        supplies.release_target(task).map_err(|_| Fail::NotReady)?;
    }
    Ok(Progress::Done)
}

pub fn retire_construction(
    active: Res<Active>,
    mut construction: ResMut<crate::system::control::Construction>,
    mut jobs: ResMut<crate::system::assembly::Jobs>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.progress.execution.task {
        construction
            .retire(task, |entry, caller| {
                jobs.retire_create_scope(entry, caller).map_err(|_| ())
            })
            .map_err(|_| Fail::NotReady)?;
    }
    Ok(Progress::Done)
}

pub fn retire_identity(active: Res<Active>, mut roster: ResMut<Roster>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.progress.execution.task {
        if let Some(identity) = job
            .progress
            .execution
            .instance
            .as_ref()
            .and_then(|instance| instance.identity_install)
            .or_else(|| roster.install_id(task))
        {
            roster
                .retire_install(identity)
                .map_err(|_| Fail::NotReady)?;
        }
    }
    Ok(Progress::Done)
}

pub fn finalize(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    control.table.detach(&job.key.request.name);
    control.table.set_state(&job.key.request.name, State::Dead);
    job.progress.execution.instance = None;
    Ok(Progress::Done)
}
