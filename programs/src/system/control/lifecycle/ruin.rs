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
    roster: Res<Roster>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    control.input(&job.request.name)?.program;
    let Some(task) = job
        .execution
        .task
        .or_else(|| control.task(&job.request.name))
    else {
        if control
            .table
            .find(&job.request.name)
            .is_some_and(|r| r.state == State::Dead)
        {
            return Ok(Progress::Done);
        }
        return Err(Fail::Unknown);
    };
    roster.unbind(task).map_err(|_| Fail::NotReady)?;
    job.execution.task = Some(task);
    control.table.set_state(&job.request.name, State::Stopping);
    Ok(Progress::Done)
}
pub fn run(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.execution.task {
        let team = match control.table.find(&job.request.name).map(|row| row.slot) {
            Some(Slot::Live { team, .. }) => team,
            _ => None,
        };
        if let Some(team) = team {
            let _ = env::unit::slay_team(team);
        }
        if !env::unit::join_task(task, Wait::POLL).unwrap_or(true) {
            if team.is_none() {
                let _ = env::room::doom(task);
            }
            if !env::unit::join_task(task, Wait::POLL).unwrap_or(true) {
                return if env::chrono::clock() < job.execution.deadline {
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
    if let Some(row) = control.table.find(&job.request.name) {
        if let Slot::Live {
            team: Some(team), ..
        } = row.slot
        {
            match env::unit::oust(team) {
                Ok(()) => {}
                Err(e)
                    if matches!(e.source, env::UnitFail::Busy)
                        && env::chrono::clock() < job.execution.deadline =>
                {
                    return Ok(Progress::Pending);
                }
                Err(e) if matches!(e.source, env::UnitFail::Denied) => {}
                Err(_) => return Err(Fail::NotReady),
            }
        }
    }
    control.pending.retain(|p| p.name != job.request.name);
    control.table.detach(&job.request.name);
    control.table.set_state(&job.request.name, State::Dead);
    job.execution.instance = None;
    Ok(Progress::Done)
}
