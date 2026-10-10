use super::super::unit::Control;
use super::{Action, Active, Instance};
use crate::system::control::unit::task::Readiness;
use crate::system::{
    control::identity::Roster,
    control::unit::{table::State, verdict::Fail},
};
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{Mark, Wait};

pub(crate) fn pre(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
    roster: Res<Roster>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let p = control.input(&job.request.name)?.program;
    if control
        .table
        .find(p.name())
        .is_some_and(|r| r.state == State::Debarked)
    {
        job.execution.task = control.task(p.name());
        return Ok(Progress::Done);
    }
    let at = control
        .pending
        .iter()
        .position(|p| p.name == job.request.name)
        .ok_or(Fail::NotReady)?;
    let pending = control.pending.remove(at);
    job.execution.task = Some(pending.service.task());
    job.execution.instance = Some(Instance {
        service: pending.service,
        marks: Vec::new(),
        launched: false,
    });
    let instance = job.execution.instance.as_mut().ok_or(Fail::NotReady)?;
    match job.request.action {
        Action::Embark {
            parent: Some(parent),
        } => roster.inherit(instance.service.task(), parent),
        Action::Embark { parent: None } => roster.authorize(instance.service.task()),
        _ => return Err(Fail::Unknown),
    }
    .map_err(|_| Fail::NotReady)?;
    if matches!(job.request.action, Action::Embark { parent: None }) {
        control.table.mark_static(instance.service.task());
    }
    instance.service.connect(p).map_err(|_| Fail::NotReady)?;
    for setup in p.supply() {
        for channel in [Some(setup.channel()), setup.ready()].into_iter().flatten() {
            instance.marks.try_reserve(1).map_err(|_| Fail::Full)?;
            instance.marks.push(Mark::of(channel));
        }
    }
    Ok(Progress::Done)
}

pub fn run(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(instance) = job.execution.instance.as_mut() {
        if !instance.launched {
            env::unit::embark_task(instance.service.task()).map_err(|_| Fail::NotReady)?;
            instance.launched = true;
        }
    } else {
        control.resume(&job.request.name)?;
    }
    Ok(Progress::Done)
}
pub fn ready(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let Some(instance) = job.execution.instance.as_mut() else {
        return Ok(Progress::Done);
    };
    let p = control.input(&job.request.name)?.program;
    match instance.service.ready(
        &mut control.table,
        Readiness {
            name: p.name(),
            marks: &instance.marks,
            wait: Wait::POLL,
        },
    ) {
        Ok(true) => Ok(Progress::Done),
        Ok(false)
            if control
                .table
                .find(p.name())
                .is_some_and(|r| r.state == State::Ready) =>
        {
            Ok(Progress::Done)
        }
        Ok(false) if env::chrono::clock() < job.execution.deadline => Ok(Progress::Pending),
        _ => Err(Fail::NotReady),
    }
}
pub fn post(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if control
        .table
        .find(&job.request.name)
        .is_some_and(|r| r.state == State::Ready)
    {
        Ok(Progress::Done)
    } else {
        Err(Fail::NotReady)
    }
}
