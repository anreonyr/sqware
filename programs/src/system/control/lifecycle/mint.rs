use super::super::unit::{

    {Control, Pending},
};
use super::{Action, Active, Operations};
use crate::system::control::unit::{
    table::{Slot, State},
    verdict::Fail,
};
use ::schedule::{Progress, Res, ResMut};

pub fn pre(
    active: Res<Active>,
    mut control: ResMut<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let p = control.input(&job.request.name)?.program;
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
    Ok(Progress::Done)
}
pub fn post(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let instance = job.execution.instance.take().ok_or(Fail::NotReady)?;
    let name = control.input(&job.request.name)?.program.name();
    control.pending.push(Pending {
        name,
        service: instance.service,
    });
    Ok(Progress::Done)
}

pub fn failed(
    operations: Res<Operations>,
    mut control: ResMut<Control>,
) -> Result<Progress, crate::system::app::Fault> {
    for tracked in &operations.0 {
        let job = &tracked.operation;
        if tracked.complete
            && job.failure.is_some()
            && matches!(job.request.action, Action::Mint)
            && job.execution.task.is_none()
            && control.table.find(&job.request.name).is_some_and(|row| {
                row.state == State::NeverStarted && matches!(row.slot, Slot::None)
            })
        {
            control.table.set_state(&job.request.name, State::Dead);
        }
    }
    Ok(Progress::Done)
}
