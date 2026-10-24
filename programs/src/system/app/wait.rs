//! Supervisor interest selection and wait policy.
pub(crate) use super::waiting::Waiting;
use crate::system::app::Fault as Fail;
use crate::system::control::{namespace::Endpoints as Watch, unit::Control, unit::table::Slot};

use alloc::vec::Vec;
use env::ToleEventSource as Source;
use env::{PieToken, Wait};
use system_api::control as ccall;

use ::schedule::{Progress, Res, ResMut};
pub struct Interests {
    pub tokens: Vec<PieToken>,
    pub writes: Vec<PieToken>,
    pub subs: Vec<Source>,
    pub armed: bool,
}
pub fn entries(watch: Res<Watch>, mut wanted: ResMut<Interests>) -> Result<Progress, Fail> {
    wanted.tokens.clear();
    wanted.writes.clear();
    wanted.subs.clear();
    wanted.armed = false;
    wanted.tokens.try_reserve(ccall::Grant::ALL.len() + 3)?;
    wanted.tokens.extend(watch.entries());
    Ok(Progress::Done)
}
pub fn tasks(control: Res<Control>, mut wanted: ResMut<Interests>) -> Result<Progress, Fail> {
    wanted
        .subs
        .try_reserve(control.living_count() + control.instances().count() * 2 + 3)?;
    wanted
        .subs
        .extend(control.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. } => Some(Source::Join {
                target: task.into(),
            }),
            _ => None,
        }));
    wanted.subs.push(Source::Join {
        target: control
            .bootstrap_task(crate::unit::Bootstrap::Operator)
            .ok_or(Fail::Dead)?
            .into(),
    });
    wanted.subs.push(Source::Join {
        target: control
            .bootstrap_task(crate::unit::Bootstrap::Identity)
            .ok_or(Fail::Dead)?
            .into(),
    });
    for item in control.instances() {
        if let Some(team) = item.team() {
            if !item.claimed {
                wanted.subs.push(Source::Join {
                    target: team.into(),
                });
            }
            wanted.subs.push(Source::Join {
                target: item.task.into(),
            });
        }
        wanted.subs.push(Source::Join {
            target: item.owner.into(),
        });
    }
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
    wanted.subs.push(Source::Nole { token: changed.0.token() });
    Ok(Progress::Done)
}

/// Admission contributes its own transport interest and expiry to the supervisor wait.
pub(super) fn connections(
    tree: Res<crate::system::operator::management::Tree>,
    mut wanted: ResMut<Interests>,
    mut bound: ResMut<super::policy::Bound>,
) -> Result<Progress, Fail> {
    let count = tree.connection_interests().count();
    wanted.tokens.try_reserve(count)?;
    wanted.writes.try_reserve(count)?;
    for (token, direction) in tree.connection_interests() {
        match direction {
            env::HoleCondition::Pull => wanted.tokens.push(token),
            env::HoleCondition::Push | env::HoleCondition::Signal => return Err(Fail::Room),
            env::HoleCondition::Empty => wanted.writes.push(token),
        }
    }
    // LINK admission scans newly delivered capabilities.
    bound.0 = match (bound.0, tree.connection_budget()) {
        (Wait::Forever, value) | (value, Wait::Forever) => value,
        (Wait::AtMost(left), Wait::AtMost(right)) => Wait::AtMost(left.min(right)),
    };
    bound.0 = match bound.0 {
        Wait::Forever => Wait::AtMost(100),
        Wait::AtMost(ms) => Wait::AtMost(ms.min(100)),
    };
    Ok(Progress::Done)
}

pub(crate) fn construction(
    construction: Res<crate::system::control::Construction>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    let entries = construction.entries();
    wanted.tokens.try_reserve(entries.size_hint().0)?;
    wanted.tokens.extend(entries);
    Ok(Progress::Done)
}
