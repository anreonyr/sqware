use crate::system::control::serve::{
    Fail,
    lifecycle::{Action, Operations, Request},
    unit::Control,
};
use crate::system::{
    control::core::{
        unit::{Slot, State},
        verdict as core,
    },
    life::{Phase, Status},
};
use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;
use alloc::vec::Vec;
use env::Wait;
use ::schedule::{Progress, Res, ResMut};

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
    if let Some(job) = operations
        .0
        .iter()
        .find(|job| job.complete && job.operation.request.back.is_none())
    {
        if job.operation.failure.is_some() {
            return Err(Fail::Shutdown);
        }
        let action = match job.operation.request.action {
            Action::Mint => Action::Embark { parent: None },
            Action::Embark { .. } => {
                startup.at += 1;
                Action::Mint
            }
            _ => return Ok(Progress::Done),
        };
        if let Some(program) = startup.list.get(startup.at) {
            operations
                .push(Request {
                    name: program.name().into(),
                    action,
                    back: None,
                })
                .map_err(|_| Fail::Room)?;
        }
    } else if operations.0.is_empty() {
        operations
            .push(Request {
                name: startup.list[startup.at].name().into(),
                action: Action::Mint,
                back: None,
            })
            .map_err(|_| Fail::Room)?;
    }
    Ok(Progress::Done)
}
pub fn reply(mut operations: ResMut<Operations>) -> Result<Progress, Fail> {
    let count = operations.0.len();
    for _ in 0..count {
        if let Some(tracked) = operations.0.pop_front() {
            if tracked.complete && tracked.operation.request.back.is_some() {
                crate::system::control::serve::answer::complete(&tracked.operation);
            } else {
                operations.0.push_back(tracked);
            }
        }
    }
    Ok(Progress::Done)
}
pub fn activity(control: Res<Control>, mut activity: ResMut<Activity>) -> Result<Progress, Fail> {
    let living = control.table.living().count();
    if activity.owed == 0 || living < activity.owed {
        activity.quiet = env::chrono::clock();
    }
    activity.owed = living;
    activity.walking = core::walking(&control.table);
    Ok(Progress::Done)
}
pub fn eligibility(control: Res<Control>, mut startup: ResMut<Startup>) -> Result<Progress, Fail> {
    startup.eligible = startup.at == startup.list.len() && core::due(&control.table);
    Ok(Progress::Done)
}
pub fn settle(
    startup: Res<Startup>,
    operations: Res<Operations>,
    mut flow: ResMut<Flow>,
) -> Result<Progress, Fail> {
    if startup.eligible && operations.0.is_empty() {
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
        for row in control.table.living() {
            if matches!(row.state, State::Starting | State::Ready | State::Debarked)
                && matches!(row.slot, Slot::Live { team: Some(_), .. })
                && !operations
                    .0
                    .iter()
                    .any(|job| job.operation.request.name == row.name)
            {
                operations
                    .push(Request {
                        name: row.name.clone(),
                        action: Action::Ruin,
                        back: None,
                    })
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
        && core::done(&control.table)
        && operations.0.is_empty()
        && control.instances.iter().all(|item| item.team.is_none());
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
    if !operations.0.is_empty() || !inbox.0.is_empty() {
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
        && operations.0.is_empty()
        && status.phase.load(Ordering::Acquire) == Phase::Starting as u8
    {
        status.phase.store(Phase::Running as u8, Ordering::Release);
        protocol::debug::put("system: static units ready; Running");
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
    operations
        .0
        .retain(|job| !job.complete || job.operation.request.back.is_some());
    Ok(Progress::Done)
}
