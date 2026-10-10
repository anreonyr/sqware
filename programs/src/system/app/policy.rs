use crate::system::app::Fault as Fail;
use crate::system::control::Startup;
use crate::system::control::lifecycle::Operations;
use crate::system::control::unit::Control;

use crate::system::app::life::{Phase, Status};
use ::core::sync::atomic::Ordering;
use ::schedule::{Progress, Res, ResMut};
use alloc::sync::Arc;
use env::Wait;

pub struct Flow {
    pub settling: bool,
    pub forced: bool,
    pub done: bool,
}
pub struct Activity {
    pub owed: usize,
    pub quiet: u64,
    pub walking: bool,
}
pub struct Bound(pub Wait);
pub struct Shutoff(pub Option<u64>);

pub fn health(status: Res<Arc<Status>>) -> Result<Progress, Fail> {
    for id in [
        status.operator.load(Ordering::Acquire),
        status.identity.load(Ordering::Acquire),
    ] {
        if env::unit::join_task(env::TaskId::new(id), Wait::POLL).unwrap_or(true) {
            return Err(Fail::Dead);
        }
    }
    Ok(Progress::Done)
}
pub fn activity(control: Res<Control>, mut activity: ResMut<Activity>) -> Result<Progress, Fail> {
    let living = control.living_count();
    if activity.owed == 0 || living < activity.owed {
        activity.quiet = env::chrono::clock();
    }
    activity.owed = living;
    activity.walking = control.walking();
    Ok(Progress::Done)
}
pub fn settle(
    startup: Res<Startup>,
    operations: Res<Operations>,
    mut flow: ResMut<Flow>,
) -> Result<Progress, Fail> {
    if startup.eligible() && operations.is_empty() {
        flow.settling = true;
    }
    Ok(Progress::Done)
}
pub fn deadline(flow: Res<Flow>, mut shutoff: ResMut<Shutoff>) -> Result<Progress, Fail> {
    if flow.settling && shutoff.0.is_none() {
        shutoff.0 = Some(env::chrono::clock());
    }
    Ok(Progress::Done)
}
pub fn idle(
    activity: Res<Activity>,
    mut flow: ResMut<Flow>,
    shutoff: Res<Shutoff>,
) -> Result<Progress, Fail> {
    let now = env::chrono::clock();
    if flow.settling {
        if shutoff
            .0
            .is_some_and(|since| now.saturating_sub(since) >= 10_000_000_000)
        {
            return Err(Fail::Idle);
        }
    } else if activity.walking && now.saturating_sub(activity.quiet) >= 10_000_000_000 {
        flow.forced = true;
        flow.settling = true;
    }
    Ok(Progress::Done)
}
pub fn done(
    control: Res<Control>,
    operations: Res<Operations>,
    mut flow: ResMut<Flow>,
) -> Result<Progress, Fail> {
    flow.done = flow.settling
        && control.done()
        && operations.is_empty()
        && control.instances().all(|item| item.team.is_none());
    if flow.done && flow.forced {
        return Err(Fail::Idle);
    }
    Ok(Progress::Done)
}
pub fn bound(
    activity: Res<Activity>,
    flow: Res<Flow>,
    mut bound: ResMut<Bound>,
) -> Result<Progress, Fail> {
    bound.0 = if flow.done {
        Wait::POLL
    } else if !flow.settling && !activity.walking {
        Wait::Forever
    } else {
        Wait::AtMost(
            (10_000_000_000u64.saturating_sub(env::chrono::clock().saturating_sub(activity.quiet)))
                .div_ceil(1_000_000)
                .max(1) as usize,
        )
    };
    Ok(Progress::Done)
}

pub fn running(
    startup: Res<Startup>,
    operations: Res<Operations>,
    status: Res<Arc<Status>>,
) -> Result<Progress, Fail> {
    if startup.complete()
        && operations.is_empty()
        && status.phase.load(Ordering::Acquire) == Phase::Starting as u8
    {
        status.phase.store(Phase::Running as u8, Ordering::Release);
        programs::debug::put("system: static units ready; Running");
    }
    Ok(Progress::Done)
}

pub fn stopping_bound(
    flow: Res<Flow>,
    shutoff: Res<Shutoff>,
    mut bound: ResMut<Bound>,
) -> Result<Progress, Fail> {
    if flow.settling && !flow.done {
        let left = shutoff.0.map_or(1_000_000, |since| {
            10_000_000_000u64.saturating_sub(env::chrono::clock().saturating_sub(since))
        });
        bound.0 = Wait::AtMost(left.div_ceil(1_000_000).max(1) as usize);
    }
    Ok(Progress::Done)
}
