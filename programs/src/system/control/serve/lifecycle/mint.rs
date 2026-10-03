use alloc::vec::Vec;
use protocol::common::schedule::{Progress, Res, ResMut};
use crate::system::control::core::{unit::State, verdict::Fail};
use super::{Active, Instance, program};
use super::super::{unit::{Control, Pending}, start::{self, Images}};

pub fn pre(active: Res<Active>, mut control: ResMut<Control>, images: Res<Images>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let p = program(&job.request.name)?;
    if images.catalog.find(p.name()).is_none() { return Err(Fail::BadImage); }
    if let Some(row) = control.table.find(p.name()) {
        if !matches!(row.state, State::NeverStarted | State::Dead) { return Err(Fail::NotReady); }
    } else { control.enlist(p).map_err(|_| Fail::Full)?; }
    control.pending.try_reserve(1).map_err(|_| Fail::Full)?;
    Ok(Progress::Done)
}
pub fn run(mut active: ResMut<Active>, mut control: ResMut<Control>, images: Res<Images>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let service = control.spawn(program(&job.request.name)?, &images).map_err(|e| match e { start::Error::Missing => Fail::BadImage, _ => Fail::Full })?;
    job.task = Some(service.0);
    job.instance = Some(Instance { service, marks: Vec::new(), launched: false });
    Ok(Progress::Done)
}
pub fn post(mut active: ResMut<Active>, mut control: ResMut<Control>, images: Res<Images>) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let instance = job.instance.as_ref().ok_or(Fail::NotReady)?;
    super::super::publication::inject(images.entry, instance.service.0).map_err(|_| Fail::Full)?;
    let instance = job.instance.take().ok_or(Fail::NotReady)?;
    control.pending.push(Pending { name: program(&job.request.name)?.name(), service: instance.service });
    Ok(Progress::Done)
}
