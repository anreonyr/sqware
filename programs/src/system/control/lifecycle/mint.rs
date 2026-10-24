use super::super::unit::{Control, Pending};
use super::{Action, Active};
use crate::system::control::unit::{
    table::{Slot, State},
    verdict::Fail,
};
use ::schedule::{Progress, Res, ResMut};

pub fn pre(active: Res<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let p = control.input(&job.key.request.name)?.program;
    if control.input(p.name())?.image.is_none() {
        return Err(Fail::BadImage);
    }
    if let Some(row) = control.table.find(p.name()) {
        if !matches!(row.state, State::NeverStarted | State::Dead) {
            return Err(Fail::NotReady);
        }
    } else {
        control.enlist(p).map_err(|_| Fail::Full)?;
    }
    control.pending.try_reserve(1).map_err(|_| Fail::Full)?;
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
pub fn post(mut active: ResMut<Active>, mut control: ResMut<Control>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let instance = job
        .progress
        .execution
        .instance
        .take()
        .ok_or(Fail::NotReady)?;
    let name = control.input(&job.key.request.name)?.program.name();
    control.pending.push(Pending {
        name,
        service: instance.service,
    });
    Ok(Progress::Done)
}

pub fn failed(
    operations: Res<crate::system::assembly::Jobs>,
    mut control: ResMut<Control>,
) -> Result<Progress, crate::system::app::Fault> {
    for tracked in operations.lifecycle_jobs() {
        let job = &tracked.operation;
        if tracked.complete
            && job.progress.failure.is_some()
            && matches!(job.key.request.directive.action, Action::Mint)
            && job.progress.execution.task.is_none()
            && control
                .table
                .find(&job.key.request.name)
                .is_some_and(|row| {
                    row.state == State::NeverStarted && matches!(row.slot, Slot::None)
                })
        {
            control.table.set_state(&job.key.request.name, State::Dead);
        }
    }
    Ok(Progress::Done)
}
