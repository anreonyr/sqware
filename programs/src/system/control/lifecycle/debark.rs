use super::super::unit::Control;
use super::Active;
use crate::system::control::unit::{
    table::{Slot, State},
    verdict::Fail,
};
use ::schedule::{Progress, Res, ResMut};

pub fn pre(mut active: ResMut<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    control.input(&job.key.request.name)?.program;
    let row = control
        .table
        .find(&job.key.request.name)
        .ok_or(Fail::Unknown)?;
    if !matches!(row.state, State::Ready | State::Debarked) {
        return Err(Fail::NotReady);
    }
    job.progress.execution.task = control.task(&job.key.request.name);
    Ok(Progress::Done)
}
pub fn run(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let row = control
        .table
        .find(&job.key.request.name)
        .ok_or(Fail::Unknown)?;
    if row.state == State::Debarked {
        return Ok(Progress::Done);
    }
    let Slot::Live {
        team: Some(team), ..
    } = row.slot
    else {
        return Err(Fail::Unknown);
    };
    match env::unit::debark_team(team) {
        Ok(()) => Ok(Progress::Done),
        Err(e)
            if matches!(e.source, env::UnitFail::Busy)
                && env::chrono::clock() < job.progress.execution.deadline =>
        {
            Ok(Progress::Pending)
        }
        _ => Err(Fail::NotReady),
    }
}
pub fn post(active: Res<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    control
        .table
        .set_state(&job.key.request.name, State::Debarked);
    Ok(Progress::Done)
}
