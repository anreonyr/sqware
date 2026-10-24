use super::super::unit::{Control, guard};
use super::{Action, Active, Instance, authority};
use crate::system::app::policy::Settle;
use crate::system::control::unit::task::Readiness;
use crate::system::{
    control::identity::Roster,
    control::unit::{table::State, verdict::Fail},
};
use ::schedule::{Progress, Res, ResMut};
use env::{Mark, Wait};

pub(crate) fn pre(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
    mut roster: ResMut<Roster>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let p = control.input(&job.key.request.name)?.program;
    crate::debug::put(&alloc::format!("system: prepare {} begin", p.name()));
    if control
        .table
        .find(p.name())
        .is_some_and(|r| r.state == State::Debarked)
    {
        job.progress.execution.task = control.task(p.name());
        match super::intent::change(
            job.key.request.directive.action,
            job.key.request.directive.origin,
        ) {
            Some(true) => control.request_run(p.name())?,
            Some(false) => control.request_stop(p.name())?,
            None => {}
        }
        return Ok(Progress::Done);
    }
    let at = control
        .pending
        .iter()
        .position(|p| p.name == job.key.request.name)
        .ok_or(Fail::NotReady)?;
    let pending = control.pending.remove(at);
    job.progress.execution.task = Some(pending.service.task());
    job.progress.execution.instance = Some(Instance {
        service: pending.service,
        launched: false,
        identity_install: None,
    });
    let instance = job
        .progress
        .execution
        .instance
        .as_mut()
        .ok_or(Fail::NotReady)?;
    let identity_install = match job.key.request.directive.action {
        Action::Embark {
            parent: Some(parent),
        } => roster.inherit(instance.service.task(), parent),
        Action::Embark { parent: None } => roster.authorize(instance.service.task()),
        _ => return Err(Fail::Unknown),
    }
    .map_err(|why| {
        crate::debug::put(&alloc::format!(
            "system: identity {} failed: {why}",
            job.key.request.name
        ));
        Fail::NotReady
    })?;
    crate::debug::put(&alloc::format!(
        "system: prepare {} identity installed",
        p.name()
    ));
    instance.identity_install = Some(identity_install);
    if matches!(
        job.key.request.directive.action,
        Action::Embark { parent: None }
    ) {
        control.table.mark_static(instance.service.task());
    }
    instance.service.connect(p).map_err(|why| {
        crate::debug::put(&alloc::format!(
            "system: channels {} failed: {why:?}",
            p.name()
        ));
        Fail::NotReady
    })?;
    match super::intent::change(
        job.key.request.directive.action,
        job.key.request.directive.origin,
    ) {
        Some(true) => control.request_run(p.name())?,
        Some(false) => control.request_stop(p.name())?,
        None => {}
    }
    Ok(Progress::Done)
}

pub fn run(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
    settle: Res<Settle>,
) -> Result<Progress, Fail> {
    let (name, task) = {
        let job = active.0.as_mut().ok_or(Fail::Unknown)?;
        let name = job.key.request.name.clone();
        let deadline = job.progress.execution.deadline;
        let Some(instance) = job.progress.execution.instance.as_mut() else {
            control.resume(&name)?;
            return Ok(Progress::Done);
        };
        if instance.launched {
            return Ok(Progress::Done);
        }
        let task = instance.service.task();
        if control.guard_required(&name) && !control.guard_held(&name) {
            let team = control.task_team(task).ok_or(Fail::Unknown)?;
            let authority = control.guard_token.ok_or(Fail::NotReady)?;
            match guard::arm(team, authority).map_err(|_| Fail::NotReady)? {
                Progress::Done => {
                    control.hold_guard(&name);
                    guard::resume(team).map_err(|_| Fail::NotReady)?;
                }
                Progress::Pending if env::chrono::clock() < deadline => {
                    return Ok(Progress::Pending);
                }
                _ => return Err(Fail::NotReady),
            }
        }
        (name, task)
    };
    authority::verify_effect(&active, &control, &settle)?;
    crate::debug::put(&alloc::format!(
        "system: activate {} tid={}",
        name,
        task.get()
    ));
    env::unit::embark_task(task).map_err(|_| Fail::NotReady)?;
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let instance = job
        .progress
        .execution
        .instance
        .as_mut()
        .ok_or(Fail::NotReady)?;
    instance.launched = true;
    Ok(Progress::Done)
}
pub fn ready(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let Some(instance) = job.progress.execution.instance.as_mut() else {
        return Ok(Progress::Done);
    };
    let p = control.input(&job.key.request.name)?.program;
    let mut ready_mark = [Mark::NONE];
    let marks = match p.control.ready {
        crate::unit::Ready::OnSpawn => &ready_mark[..0],
        crate::unit::Ready::Signal(mark) => {
            ready_mark[0] = mark;
            &ready_mark[..1]
        }
    };
    match instance.service.ready(
        &mut control.table,
        Readiness {
            name: p.name(),
            marks,
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
        Ok(false) if env::chrono::clock() < job.progress.execution.deadline => {
            Ok(Progress::Pending)
        }
        _ => Err(Fail::NotReady),
    }
}
pub fn post(active: Res<Active>, control: Res<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    if control
        .table
        .find(&job.key.request.name)
        .is_some_and(|r| r.state == State::Ready)
    {
        Ok(Progress::Done)
    } else {
        Err(Fail::NotReady)
    }
}
