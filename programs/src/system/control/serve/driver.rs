use super::{answer, lifecycle::{Action, Active, Operations, Request}, material::Supplies,
    start::{self, Images}, unit::Control};
use crate::service::hub::bridge::Activation;
use crate::system::{control::core::{unit::{Slot, State}, verdict::Fail},
    identity::serve::install::Roster, run::schedule::Lifecycle};
use protocol::common::schedule::{Resources, Progress, RunError};

pub(crate) fn poll(plans: &mut Lifecycle, operations: &mut Operations,
    control: &mut Control, roster: &Roster, supplies: &mut Supplies,
    activation: &mut Option<Activation>, images: &Images) -> Result<(), super::Fail> {
    let rounds = operations.0.len();
    for _ in 0..rounds {
        let Some(mut tracked) = operations.0.pop_front() else { break; };
        if tracked.complete { operations.0.push_back(tracked); continue; }
        let mut active = Active(Some(tracked.operation));
        let result = {
            let mut resources = Resources::new();
            resources.borrow(control).map_err(|_| super::Fail::Room)?;
            resources.observe(roster).map_err(|_| super::Fail::Room)?;
            resources.observe(images).map_err(|_| super::Fail::Room)?;
            resources.borrow(supplies).map_err(|_| super::Fail::Room)?;
            resources.borrow(activation).map_err(|_| super::Fail::Room)?;
            resources.borrow(&mut active).map_err(|_| super::Fail::Room)?;
            let plan = match active_action(&resources)? {
                Action::Mint => &mut plans.mint,
                Action::Embark { .. } => &mut plans.embark,
                Action::Debark => &mut plans.debark,
                Action::Ruin => &mut plans.ruin,
            };
            plan.advance(&mut tracked.cursor, &resources)
        };
        tracked.operation = active.0.take().ok_or(super::Fail::Room)?;
        match result {
            Ok(Progress::Done) => tracked.complete = true,
            Ok(Progress::Pending) => {},
            Err(error) => {
                let fail = match error { RunError::Step(fail) => fail, RunError::Resource(_) => Fail::Full };
                let job = &mut tracked.operation;
                if matches!(job.request.action, Action::Ruin) && job.task.is_some() { return Err(super::Fail::Shutdown); }
                job.failure = Some(fail);
                if matches!(job.request.action, Action::Mint) && job.task.is_none()
                    && control.table.find(&job.request.name).is_some_and(|row| row.state == State::NeverStarted && matches!(row.slot, Slot::None)) {
                    control.table.set_state(&job.request.name, State::Dead);
                }
                if matches!(job.request.action, Action::Mint | Action::Embark { .. }) && job.instance.is_some() {
                    job.request.action = Action::Ruin;
                    job.deadline = runtime::env::chrono::clock() + start::BOOT_MS as u64 * 1_000_000;
                    tracked.cursor.reset();
                } else { tracked.complete = true; }
            }
        }
        operations.0.push_back(tracked);
    }
    Ok(())
}
fn active_action(resources: &Resources<'_>) -> Result<Action, super::Fail> {
    resources.read::<Active>().map_err(|_| super::Fail::Room)?.0.as_ref()
        .map(|job| job.request.action).ok_or(super::Fail::Room)
}
pub(super) fn reply(operations: &mut Operations) {
    let rounds = operations.0.len();
    for _ in 0..rounds {
        if let Some(tracked) = operations.0.pop_front() {
            if tracked.complete { answer::complete(&tracked.operation); }
            else { operations.0.push_back(tracked); }
        }
    }
}
pub(super) fn ruin_rest(control: &Control, operations: &mut Operations) -> Result<(), Fail> {
    for row in control.table.living() {
        if matches!(row.state, State::Starting | State::Ready | State::Debarked)
            && matches!(row.slot, Slot::Live { team: Some(_), .. })
            && !operations.0.iter().any(|job| job.operation.request.name == row.name) {
            operations.push(Request { name: row.name.clone(), action: Action::Ruin, back: None })?;
        }
    }
    Ok(())
}
