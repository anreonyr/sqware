use crate::system::control::lifecycle::{Action, Operations};
use crate::system::control::serve::{Fail, unit::Control};
use crate::system::life::{Phase, Status};
use ::core::sync::atomic::Ordering;
use ::schedule::{Progress, Res, ResMut};
use alloc::sync::Arc;
use alloc::vec::Vec;
use env::Wait;

pub struct Startup {
    pub list: Vec<&'static crate::unit::UnitFile>,
    pub at: usize,
    pub eligible: bool,
}
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
        if env::unit::join(env::TaskId::new(id), Wait::POLL).unwrap_or(true) {
            return Err(Fail::Dead);
        }
    }
    Ok(Progress::Done)
}
pub fn startup(
    mut startup: ResMut<Startup>,
    mut operations: ResMut<Operations>,
    flow: Res<Flow>,
) -> Result<Progress, Fail> {
    if flow.settling || startup.at == startup.list.len() {
        return Ok(Progress::Done);
    }
    if let Some(result) = operations.completed_local_action() {
        let completed = result.map_err(|_| Fail::Shutdown)?;
        let action = match completed {
            Action::Mint => Action::Embark { parent: None },
            Action::Embark { .. } => {
                startup.at += 1;
                Action::Mint
            }
            _ => return Ok(Progress::Done),
        };
        if let Some(program) = startup.list.get(startup.at) {
            operations
                .submit(program.name().into(), action)
                .map_err(|_| Fail::Room)?;
        }
    } else if operations.is_empty() {
        operations
            .submit(startup.list[startup.at].name().into(), Action::Mint)
            .map_err(|_| Fail::Room)?;
    }
    Ok(Progress::Done)
}
pub fn reply(mut operations: ResMut<Operations>) -> Result<Progress, Fail> {
    operations.reply_completed();
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
pub fn eligibility(control: Res<Control>, mut startup: ResMut<Startup>) -> Result<Progress, Fail> {
    startup.eligible = startup.at == startup.list.len() && control.due();
    Ok(Progress::Done)
}
pub fn settle(
    startup: Res<Startup>,
    operations: Res<Operations>,
    mut flow: ResMut<Flow>,
) -> Result<Progress, Fail> {
    if startup.eligible && operations.is_empty() {
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
pub fn ruin_rest(
    control: Res<Control>,
    flow: Res<Flow>,
    mut operations: ResMut<Operations>,
) -> Result<Progress, Fail> {
    if flow.settling {
        for name in control.closing_service_names() {
            if !operations.contains(name) {
                operations
                    .submit(name.into(), Action::Ruin)
                    .map_err(|_| Fail::Room)?;
            }
        }
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
pub fn pending(
    operations: Res<Operations>,
    inbox: Res<crate::system::control::serve::answer::Inbox>,
    mut bound: ResMut<Bound>,
) -> Result<Progress, Fail> {
    if !operations.is_empty() || !inbox.0.is_empty() {
        bound.0 = Wait::AtMost(1);
    }
    Ok(Progress::Done)
}
pub fn running(
    startup: Res<Startup>,
    operations: Res<Operations>,
    status: Res<Arc<Status>>,
) -> Result<Progress, Fail> {
    if startup.at == startup.list.len()
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

pub fn retire_static(mut operations: ResMut<Operations>) -> Result<Progress, Fail> {
    operations.retire_local_completed();
    Ok(Progress::Done)
}
