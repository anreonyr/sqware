use super::super::unit::Control;
use super::Active;
use crate::service::hub::bridge::Activation;
use crate::system::{
    control::core::{
        unit::{Slot, State},
        verdict::Fail,
    },
    control::identity::Roster,
};
use ::schedule::{Progress, Res, ResMut};
use env::Wait;

pub(crate) fn pre(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
    roster: Res<Roster>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    super::super::unit::start::program_of(&job.request.name)?;
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
pub fn run(active: Res<Active>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.execution.task {
        if !env::unit::join(task, Wait::POLL).unwrap_or(true) {
            let _ = env::unit::slay(task);
            if !env::unit::join(task, Wait::POLL).unwrap_or(true) {
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

pub fn activation(
    active: Res<Active>,
    mut activation: ResMut<Option<Activation>>,
) -> Result<Progress, Fail> {
    if active
        .0
        .as_ref()
        .is_some_and(|job| job.request.name == "hub")
    {
        *activation = None;
    }
    Ok(Progress::Done)
}
