use super::{Action, Origin, Request, Tracked};
use crate::system::control::endpoint::request::reply;
use crate::system::control::unit::{start::BOOT_MS, verdict::Fail};
use system_api::control::{Fail as WireFail, frame};

/// Resolve a same-name lifecycle request against the in-flight job.
///
/// Returns `true` when the request took the job over, `false` when a duplicate
/// Ruin was merged. A rejected request remains unchanged.
pub(crate) fn accept(tracked: &mut Tracked, request: &mut Request) -> Result<bool, Fail> {
    if matches!(
        tracked.operation.key.request.directive.action,
        Action::Ruin
    ) && matches!(request.directive.action, Action::Ruin)
    {
        if let Some(back) = request.back.take() {
            if tracked.operation.key.waiters.try_reserve(1).is_err() {
                request.back = Some(back);
                return Err(Fail::Full);
            }
            tracked.operation.key.waiters.push(back);
        }
        crate::debug::put(&alloc::format!(
            "system: ruin merged target={} pending_origin={:?}",
            tracked.operation.key.request.name,
            tracked.operation.key.request.directive.origin,
        ));
        return Ok(false);
    }
    if !candidate(tracked)
        || request.directive.origin != Origin::External
        || request.directive.action != Action::Ruin
    {
        return Err(Fail::NotReady);
    }
    let cancelled = frame::said_status(frame::fail_to_code(Some(WireFail::NotReady)));
    let previous_back = tracked.operation.key.request.back.take();
    let previous_waiters = core::mem::take(&mut tracked.operation.key.waiters);
    tracked.operation.key.request.directive.action = Action::Ruin;
    tracked.operation.key.request.directive.origin = Origin::External;
    tracked.operation.key.request.back = request.back.take();
    tracked.cursor.reset();
    tracked.operation.progress.execution.deadline =
        env::chrono::clock() + BOOT_MS as u64 * 1_000_000;
    if let Some(back) = previous_back {
        reply(back, cancelled);
    }
    for waiter in previous_waiters {
        reply(waiter, cancelled);
    }
    Ok(true)
}

fn candidate(tracked: &Tracked) -> bool {
    matches!(
        tracked.operation.key.request.directive.origin,
        Origin::Recovery { .. }
    ) && matches!(
        tracked.operation.key.request.directive.action,
        Action::Recover | Action::Embark { parent: None }
    )
}
