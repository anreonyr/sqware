//! Instance lifecycle extension points executed as bounded schedule subplans.
use crate::system::app::Fault as Fail;
use crate::system::control::unit::Control;
use crate::system::control::unit::table::State;

use ::schedule::{BuildError, Dispatch, Invocation, Plan, Progress, Res, ResMut, Schedule};
use env::{Wait, unit};
#[derive(Clone, Copy, PartialEq)]
pub enum Key {
    Prepare,
    Retire,
}
pub fn plan(
    children: alloc::vec::Vec<(Key, Plan<&'static str>)>,
) -> Result<Plan<Fail>, BuildError> {
    let mut plan = Schedule::sequence();
    plan.system("budget", budget)?;
    plan.subplans("hooks", select, children, finish)?;
    plan.build()
}
pub fn budget(
    control: Res<Control>,
    mut jobs: ResMut<crate::system::assembly::Jobs>,
    mut dispatch: ResMut<Dispatch<Key, &'static str>>,
) -> Result<Progress, Fail> {
    jobs.reset_hook_scan();
    dispatch.begin(control.instances.len())?;
    Ok(Progress::Done)
}
pub fn select(
    control: Res<Control>,
    mut jobs: ResMut<crate::system::assembly::Jobs>,
    mut dispatch: ResMut<Dispatch<Key, &'static str>>,
) -> Result<Progress, Fail> {
    while let Some((task, team, state, started, expected)) = control
        .instances
        .get(jobs.next_hook())
        .map(|item| (item.task, item.team, item.state, item.started, item.reference))
    {
        let next = jobs.next_hook() + 1;
        jobs.advance_hook(next);
        let Some(reference) = jobs.instance_ref(task) else {
            continue;
        };
        if expected != reference {
            continue;
        }
        let key = match state {
            State::Starting if !started => Some(Key::Prepare),
            State::Stopping
                if team.is_some_and(|team| {
                    matches!(
                        unit::join(env::UnitTarget::Team(team), Wait::POLL, false),
                        Ok(env::JoinReply::Complete)
                    )
                }) => Some(Key::Retire),
            _ => None,
        };
        let Some(key) = key else { continue };
        let job = jobs.phase(reference, key).map_err(|_| Fail::Room)?;
        jobs.select_hook(crate::system::assembly::HookSelection {
            reference,
            job,
            next,
        });
        let cursor = jobs.phase_cursor(job).ok_or(Fail::Room)?;
        dispatch.select(Invocation {
            key,
            cursor: core::mem::take(cursor),
        })?;
        return Ok(Progress::Done);
    }
    jobs.clear_hook();
    dispatch.stop()?;
    Ok(Progress::Done)
}
pub fn finish(
    mut control: ResMut<Control>,
    mut jobs: ResMut<crate::system::assembly::Jobs>,
    mut dispatch: ResMut<Dispatch<Key, &'static str>>,
) -> Result<Progress, Fail> {
    let completion = dispatch.take_result()?;
    let invocation = completion.invocation;
    let reference = jobs.active_hook_reference().ok_or(Fail::Room)?;
    let job = jobs.active_hook_job().ok_or(Fail::Room)?;
    let item = control
        .instances
        .iter_mut()
        .find(|item| item.reference == reference)
        .ok_or(Fail::Room)?;
    match completion.result {
        Ok(Progress::Pending) => {
            *jobs.phase_cursor(job).ok_or(Fail::Room)? = invocation.cursor;
        }
        Ok(Progress::Done) => {
            let outcome = match invocation.key {
                Key::Prepare => crate::system::assembly::JobOutcome::Prepare(reference),
                Key::Retire => crate::system::assembly::JobOutcome::Retire(reference),
            };
            if !jobs.complete_phase(job, Ok(outcome)) {
                return Err(Fail::Room);
            }
            let result = jobs.take(job).ok_or(Fail::Room)?.result.outcome;
            match (invocation.key, result) {
                (Key::Prepare, Ok(crate::system::assembly::JobOutcome::Prepare(done)))
                    if done == reference =>
                {
                    item.state = State::Debarked;
                    item.claim_until =
                        env::chrono::clock() + system_api::loader::CLAIM_MS as u64 * 1_000_000;
                    if !jobs.complete_create_prepare(job, reference) {
                        return Err(Fail::Room);
                    }
                }
                (Key::Retire, Ok(crate::system::assembly::JobOutcome::Retire(done)))
                    if done == reference =>
                {
                    let reclaimed = item.team.take().is_some();
                    item.state = State::Dead;
                    if reclaimed {
                        control.reclaimed_at = env::chrono::clock();
                    }
                }
                _ => return Err(Fail::Room),
            }
        }
        Err(error) => {
            let failure = match error {
                ::schedule::RunError::Step(reason) => {
                    crate::system::assembly::JobFailure::Installation(reason)
                }
                _ => crate::system::assembly::JobFailure::Installation("hook dispatch"),
            };
            if !jobs.complete_phase(job, Err(failure)) {
                return Err(Fail::Room);
            }
            let failure = jobs
                .take(job)
                .ok_or(Fail::Room)?
                .result
                .outcome
                .err()
                .ok_or(Fail::Room)?;
            programs::debug::put(&alloc::format!(
                "control: instance {} hook failed {:?}",
                item.task.get(),
                match failure {
                    crate::system::assembly::JobFailure::Installation(reason) => reason,
                    crate::system::assembly::JobFailure::Construction(_) => "construction failure",
                    crate::system::assembly::JobFailure::Control(_) => "control failure",
                    crate::system::assembly::JobFailure::Command(_) => "command failure",
                }
            ));
            item.stop();
        }
    }
    jobs.clear_hook();
    Ok(Progress::Done)
}
pub fn reclaim(
    mut control: ResMut<Control>,
    active: Res<crate::system::assembly::Jobs>,
) -> Result<Progress, &'static str> {
    let reference = active
        .active_hook_reference()
        .ok_or("instance hook reference")?;
    let item = control
        .instances
        .iter_mut()
        .find(|item| item.reference == reference)
        .ok_or("instance hook target")?;
    let Some(team) = item.team else {
        return Ok(Progress::Done);
    };
    for at in 0..64 {
        match unit::join(env::UnitTarget::Team(team), Wait::POLL, true) {
            Ok(env::JoinReply::Reaped(exit)) => {
                item.reap(exit);
                // Do not let Oust discard results beyond this round's budget.
                if at == 63 {
                    return Ok(Progress::Pending);
                }
            }
            _ => break,
        }
    }
    // A live delegated owner retains its subtree receipt, even before Loader Claim.
    if (item.claimed || item.owner != unit::self_id())
        && matches!(unit::join_task(item.owner, Wait::POLL), Ok(false))
    {
        return Ok(Progress::Done);
    }
    match unit::oust(team) {
        Ok(()) => Ok(Progress::Done),
        Err(error) if error.source == env::UnitFail::Busy => Ok(Progress::Pending),
        Err(error) if error.source == env::UnitFail::Denied => Ok(Progress::Done),
        Err(_) => Err("instance reclaim"),
    }
}
