use alloc::vec::Vec;
use env::{Mark, Wait};
use protocol::common::schedule::{Progress, Res, ResMut};
use crate::system::{control::core::{unit::State, verdict::Fail}, identity::serve::install::Roster};
use crate::service::hub::bridge::Activation;
use super::{Action, Active, Instance, program};
use super::super::{unit::Control, start, material::Supplies, task};

pub fn pre(mut active: ResMut<Active>, mut control: ResMut<Control>, roster: Res<Roster>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let p = program(&job.request.name)?;
    if control.table.find(p.name()).is_some_and(|r| r.state == State::Debarked) {
        job.task = control.task(p.name());
        return Ok(Progress::Done);
    }
    let at = control.pending.iter().position(|p| p.name == job.request.name).ok_or(Fail::NotReady)?;
    let pending = control.pending.remove(at);
    job.task = Some(pending.service.0);
    job.instance = Some(Instance { service: pending.service, marks: Vec::new(), launched: false });
    let instance = job.instance.as_mut().ok_or(Fail::NotReady)?;
    match job.request.action {
        Action::Embark { parent: Some(parent) } => roster.inherit(instance.service.0, parent),
        Action::Embark { parent: None } => roster.authorize(instance.service.0),
        _ => return Err(Fail::Unknown),
    }.map_err(|_| Fail::NotReady)?;
    if matches!(job.request.action, Action::Embark { parent: None }) { control.table.mark_static(instance.service.0); }
    start::connect_all(p, &mut instance.service).map_err(|_| Fail::NotReady)?;
    for setup in p.supply() {
        for channel in [Some(setup.channel()), setup.ready()].into_iter().flatten() {
            instance.marks.try_reserve(1).map_err(|_| Fail::Full)?;
            instance.marks.push(Mark::of(channel));
        }
    }
    Ok(Progress::Done)
}
pub fn run(mut active: ResMut<Active>, mut control: ResMut<Control>, mut supplies: ResMut<Supplies>, mut activation: ResMut<Option<Activation>>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if job.instance.is_none() {
        control.resume(&job.request.name)?;
        return Ok(Progress::Done);
    }
    let p = program(&job.request.name)?;
    let instance = job.instance.as_mut().ok_or(Fail::NotReady)?;
    if !instance.launched {
        control.launch(p, job.request.name.clone(), &mut instance.service, &mut supplies, &mut activation).map_err(|_| Fail::NotReady)?;
        instance.launched = true;
    }
    match task::ready(&mut control.table, p.name(), &mut instance.service.1, &instance.marks, Wait::POLL) {
        Ok(true) => Ok(Progress::Done),
        Ok(false) if control.table.find(p.name()).is_some_and(|r| r.state == State::Ready) => Ok(Progress::Done),
        Ok(false) if runtime::env::chrono::clock() < job.deadline => Ok(Progress::Pending),
        _ => Err(Fail::NotReady),
    }
}
pub fn post(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if control.table.find(&job.request.name).is_some_and(|r| r.state == State::Ready) { Ok(Progress::Done) } else { Err(Fail::NotReady) }
}
