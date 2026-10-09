//! Supervisor interest selection and wait policy.
pub(crate) use super::waiting::Waiting;
use crate::system::app::Fault as Fail;
use crate::system::control::{Entries as Watch, unit::Control, unit::table::Slot};
use crate::system::publication::Names;

use ::resource::pile::Sub;
use alloc::vec::Vec;
use env::{PieToken, Wait};
use system_api::control as ccall;

use ::schedule::{Progress, Res, ResMut};
pub struct Interests {
    pub tokens: Vec<PieToken>,
    pub writes: Vec<PieToken>,
    pub subs: Vec<Sub>,
    pub armed: bool,
}
pub fn entries(
    watch: Res<Watch>,
    names: Res<Names>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.clear();
    wanted.writes.clear();
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
    entry: Res<crate::system::publication::Entry>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.push(entry.0);
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
pub fn apply(
    mut waiting: ResMut<Waiting>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.armed = waiting.apply((&wanted.tokens, &wanted.writes), &wanted.subs);
    Ok(Progress::Done)
}
pub fn wait(
    waiting: Res<Waiting>,
    wanted: Res<Interests>,
    bound: Res<super::policy::Bound>,
) -> Result<Progress, Fail> {
    waiting
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

/// Admission contributes its own transport interest and expiry to the supervisor wait.
pub(super) fn connections(
    tree: Res<crate::system::operator::management::Tree>,
    mut wanted: ResMut<Interests>,
    mut bound: ResMut<super::policy::Bound>,
) -> Result<Progress, Fail> {
    let count = tree.connection_interests().count();
    wanted.tokens.try_reserve(count).map_err(|_| Fail::Room)?;
    wanted.writes.try_reserve(count).map_err(|_| Fail::Room)?;
    for (token, direction) in tree.connection_interests() {
        match direction {
            env::MailCondition::Pull => wanted.tokens.push(token),
            env::MailCondition::Push | env::MailCondition::Signal(_) => return Err(Fail::Room),
            env::MailCondition::Empty => wanted.writes.push(token),
        }
    }
    bound.0 = match (bound.0, tree.connection_budget()) {
        (Wait::Forever, value) | (value, Wait::Forever) => value,
        (Wait::AtMost(left), Wait::AtMost(right)) => Wait::AtMost(left.min(right)),
    };
    Ok(Progress::Done)
}

pub(crate) fn construction(
    construction: Res<crate::system::control::Construction>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.push(construction.entry);
    Ok(Progress::Done)
}
