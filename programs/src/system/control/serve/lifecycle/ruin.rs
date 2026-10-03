use env::Wait;
use protocol::common::schedule::{Progress, Res, ResMut};
use crate::system::{control::core::{unit::{Slot, State}, verdict::Fail}, identity::serve::install::Roster};
use crate::service::hub::bridge::Activation;
use super::{Active, program};
use super::super::unit::Control;

pub fn pre(mut active: ResMut<Active>, mut control: ResMut<Control>, roster: Res<Roster>, mut activation: ResMut<Option<Activation>>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    program(&job.request.name)?;
    let Some(task) = job.task.or_else(|| control.task(&job.request.name)) else {
        if control.table.find(&job.request.name).is_some_and(|r| r.state == State::Dead) { return Ok(Progress::Done); }
        return Err(Fail::Unknown);
    };
    if job.request.name == "hub" { *activation = None; }
    roster.unbind(task).map_err(|_| Fail::NotReady)?;
    job.task = Some(task);
    control.table.set_state(&job.request.name, State::Stopping);
    Ok(Progress::Done)
}
pub fn run(active: Res<Active>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if let Some(task) = job.task {
        if !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true) {
            let _ = runtime::env::unit::slay(task);
            if !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true) {
                return if runtime::env::chrono::clock() < job.deadline { Ok(Progress::Pending) } else { Err(Fail::NotReady) };
            }
        }
    }
    Ok(Progress::Done)
}
pub fn post(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(row) = control.table.find(&job.request.name) {
        if let Slot::Live { team: Some(team), .. } = row.slot {
            match runtime::env::unit::oust(team) {
                Ok(()) => {},
                Err(e) if matches!(e.source, env::UnitFail::Busy) && runtime::env::chrono::clock() < job.deadline => return Ok(Progress::Pending),
                Err(e) if matches!(e.source, env::UnitFail::Denied) => {},
                Err(_) => return Err(Fail::NotReady),
            }
        }
    }
    control.pending.retain(|p| p.name != job.request.name);
    control.table.detach(&job.request.name);
    control.table.set_state(&job.request.name, State::Dead);
    job.instance = None;
    Ok(Progress::Done)
}
