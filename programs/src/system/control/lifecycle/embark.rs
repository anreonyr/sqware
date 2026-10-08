use super::super::serve::{material::Supplies, start, task, unit::Control};
use super::{Action, Active, Instance};
use crate::service::hub::bridge::Activation;
use crate::system::control::serve::task::Readiness;
use crate::system::{
    control::core::{unit::State, verdict::Fail},
    control::identity::Roster,
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
    let p = super::super::serve::start::program_of(&job.request.name)?;
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
    job.execution.task = Some(pending.service.0);
    job.execution.instance = Some(Instance {
        service: pending.service,
        marks: Vec::new(),
        launched: false,
    });
    let instance = job.execution.instance.as_mut().ok_or(Fail::NotReady)?;
    match job.request.action {
        Action::Embark {
            parent: Some(parent),
        } => roster.inherit(instance.service.0, parent),
        Action::Embark { parent: None } => roster.authorize(instance.service.0),
        _ => return Err(Fail::Unknown),
    }
    .map_err(|_| Fail::NotReady)?;
    if matches!(job.request.action, Action::Embark { parent: None }) {
        control.table.mark_static(instance.service.0);
    }
    start::connect_all(p, &mut instance.service).map_err(|_| Fail::NotReady)?;
    for setup in p.supply() {
        for channel in [Some(setup.channel()), setup.ready()].into_iter().flatten() {
            instance.marks.try_reserve(1).map_err(|_| Fail::Full)?;
            instance.marks.push(Mark::of(channel));
        }
    }
    Ok(Progress::Done)
}
pub fn activation(
    active: Res<Active>,
    mut activation: ResMut<Option<Activation>>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if job
        .execution
        .instance
        .as_ref()
        .is_some_and(|instance| !instance.launched)
        && job.request.name == "hub"
    {
        *activation = Some(
            Activation::open(job.execution.task.ok_or(Fail::Unknown)?)
                .map_err(|_| Fail::NotReady)?,
        );
    }
    Ok(Progress::Done)
}
pub fn run(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(instance) = job.execution.instance.as_mut() {
        if !instance.launched {
            env::unit::embark(instance.service.0).map_err(|_| Fail::NotReady)?;
            instance.launched = true;
        }
    } else {
        control.resume(&job.request.name)?;
    }
    Ok(Progress::Done)
}
pub fn supply(
    mut active: ResMut<Active>,
    mut supplies: ResMut<Supplies>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(instance) = job.execution.instance.as_mut() {
        let program = super::super::serve::start::program_of(&job.request.name)?;
        if program.supply().iter().any(|setup| setup.machine()) {
            supplies
                .enroll(&mut instance.service, program)
                .map_err(|_| Fail::NotReady)?;
        }
    }
    Ok(Progress::Done)
}
pub fn ready(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let Some(instance) = job.execution.instance.as_mut() else {
        return Ok(Progress::Done);
    };
    let p = super::super::serve::start::program_of(&job.request.name)?;
    match task::ready(
        &mut control.table,
        Readiness {
            name: p.name(),
            marks: &instance.marks,
            wait: Wait::POLL,
        },
        &mut instance.service.1,
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
