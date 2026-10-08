//! Supervisor interest selection and wait policy.
use super::names::Names;
use crate::system::control::{
    core::unit::Slot,
    serve::{Fail, unit::Control, watch::Watch},
};
use ::resource::pile::Sub;
use alloc::vec::Vec;
use env::{PieToken, Wait};
use system_api::control as ccall;

use ::schedule::{Progress, Res, ResMut};
pub struct Interests {
    pub tokens: Vec<PieToken>,
    pub subs: Vec<Sub>,
    pub armed: bool,
}
pub fn entries(
    watch: Res<Watch>,
    names: Res<Names>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.clear();
    wanted.subs.clear();
    wanted.armed = false;
    wanted
        .tokens
        .try_reserve(ccall::Grant::ALL.len() + names.entries().len() + 3)
        .map_err(|_| Fail::Room)?;
    wanted.tokens.extend(watch.entries());
    wanted.tokens.extend(names.entries());
    Ok(Progress::Done)
}
pub fn publication(
    images: Res<crate::system::control::serve::start::Images>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.push(images.entry);
    Ok(Progress::Done)
}
pub fn activation(
    activation: Res<Option<crate::service::hub::bridge::Activation>>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    if let Some(activation) = &*activation {
        wanted.tokens.push(activation.entry());
    }
    Ok(Progress::Done)
}
pub fn tasks(control: Res<Control>, mut wanted: ResMut<Interests>) -> Result<Progress, Fail> {
    wanted
        .subs
        .try_reserve(control.living_count() + control.instances().count() * 2 + 3)
        .map_err(|_| Fail::Room)?;
    wanted
        .subs
        .extend(control.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. } => Some(Sub::TaskCompleted(task)),
            _ => None,
        }));
    wanted.subs.push(Sub::TaskCompleted(
        control.task("operator").ok_or(Fail::Dead)?,
    ));
    wanted.subs.push(Sub::TaskCompleted(
        control.task("identity").ok_or(Fail::Dead)?,
    ));
    for item in control.instances() {
        if item.team.is_some() {
            wanted.subs.push(Sub::TaskCompleted(item.task));
        }
        wanted.subs.push(Sub::TaskCompleted(item.owner));
    }
    wanted.subs.push(Sub::Capabilities);
    Ok(Progress::Done)
}
pub fn apply(mut watch: ResMut<Watch>, mut wanted: ResMut<Interests>) -> Result<Progress, Fail> {
    wanted.armed = watch.apply(&wanted.tokens, &wanted.subs);
    Ok(Progress::Done)
}
pub fn wait(
    watch: Res<Watch>,
    wanted: Res<Interests>,
    bound: Res<super::frame::Bound>,
) -> Result<Progress, Fail> {
    watch
        .pile
        .await_(if wanted.armed {
            bound.0
        } else {
            Wait::AtMost(10)
        })
        .map_err(|_| Fail::Wait)?;
    Ok(Progress::Done)
}

pub fn identity_changes(
    changed: Res<crate::system::identity::revision::Changed>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.push(changed.0.token());
    Ok(Progress::Done)
}
