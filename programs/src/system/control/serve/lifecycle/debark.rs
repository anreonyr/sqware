use super::super::unit::Control;
use super::Active;
use crate::system::control::core::{unit::State, verdict::Fail};
use protocol::common::schedule::{Progress, Res, ResMut};

pub fn pre(mut active: ResMut<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    super::super::start::program_of(&job.request.name)?;
    let row = control.table.find(&job.request.name).ok_or(Fail::Unknown)?;
    if !matches!(row.state, State::Ready | State::Debarked) {
        return Err(Fail::NotReady);
    }
    job.execution.task = control.task(&job.request.name);
    Ok(Progress::Done)
}
pub fn run(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if control
        .table
        .find(&job.request.name)
        .is_some_and(|r| r.state == State::Debarked)
    {
        return Ok(Progress::Done);
    }
    match env::unit::debark(job.execution.task.ok_or(Fail::Unknown)?) {
        Ok(()) => Ok(Progress::Done),
        Err(e)
            if matches!(e.source, env::UnitFail::Busy)
                && env::chrono::clock() < job.execution.deadline =>
        {
            Ok(Progress::Pending)
        }
        _ => Err(Fail::NotReady),
    }
}
pub fn post(active: Res<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    control.table.set_state(&job.request.name, State::Debarked);
    Ok(Progress::Done)
}
