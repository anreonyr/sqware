use super::super::serve::{
    start::{self, Images},
    unit::{Control, Pending},
};
use super::{Action, Active, Instance, Operations};
use crate::system::control::core::{
    unit::{Slot, State},
    verdict::Fail,
};
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;

pub fn pre(
    active: Res<Active>,
    mut control: ResMut<Control>,
    images: Res<Images>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let p = super::super::serve::start::program_of(&job.request.name)?;
    if images.catalog.find(p.name()).is_none() {
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
pub fn run(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
    images: Res<Images>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let service = control
        .spawn(
            super::super::serve::start::program_of(&job.request.name)?,
            &images,
        )
        .map_err(|e| match e {
            start::Error::Missing => Fail::BadImage,
            _ => Fail::Full,
        })?;
    job.execution.task = Some(service.0);
    job.execution.instance = Some(Instance {
        service,
        marks: Vec::new(),
        launched: false,
    });
    Ok(Progress::Done)
}
pub fn post(
    mut active: ResMut<Active>,
    mut control: ResMut<Control>,
    images: Res<Images>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let instance = job.execution.instance.as_ref().ok_or(Fail::NotReady)?;
    images.inject(instance.service.0).map_err(|_| Fail::Full)?;
    let instance = job.execution.instance.take().ok_or(Fail::NotReady)?;
    control.pending.push(Pending {
        name: super::super::serve::start::program_of(&job.request.name)?.name(),
        service: instance.service,
    });
    Ok(Progress::Done)
}

pub fn failed(
    operations: Res<Operations>,
    mut control: ResMut<Control>,
) -> Result<Progress, super::super::serve::Fail> {
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
