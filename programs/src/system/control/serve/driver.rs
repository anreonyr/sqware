use super::{
    lifecycle::{Action, Active, Key, Operations, Tracked},
    start,
    unit::Control,
};
use crate::system::control::core::{
    unit::{Slot, State},
    verdict::Fail,
};
use ::schedule::{Dispatch, Invocation, Progress, Res, ResMut, RunError};

pub fn budget(
    operations: Res<Operations>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, super::Fail> {
    dispatch.begin(operations.0.len()).map_err(|_| super::Fail::Room)?;
    Ok(Progress::Done)
}
pub fn select(
    mut operations: ResMut<Operations>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, super::Fail> {
    let tracked = loop {
        let Some(tracked) = operations.0.pop_front() else {
            dispatch.stop().map_err(|_| super::Fail::Room)?;
            return Ok(Progress::Done);
        };
        if !tracked.complete {
            break tracked;
        }
        operations.0.push_back(tracked);
        dispatch.skip().map_err(|_| super::Fail::Room)?;
        if dispatch.remaining() == 0 {
            return Ok(Progress::Done);
        }
    };
    let key = match tracked.operation.request.action {
        Action::Mint => Key::Mint,
        Action::Embark { .. } => Key::Embark,
        Action::Debark => Key::Debark,
        Action::Ruin => Key::Ruin,
    };
    active.0 = Some(tracked.operation);
    dispatch.select(Invocation {
        key,
        cursor: tracked.cursor,
    }).map_err(|_| super::Fail::Room)?;
    Ok(Progress::Done)
}
pub fn finish(
    mut operations: ResMut<Operations>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, super::Fail> {
    let completion = dispatch.take_result().map_err(|_| super::Fail::Room)?;
    let invocation = completion.invocation;
    let mut tracked = Tracked {
        operation: active.0.take().ok_or(super::Fail::Room)?,
        cursor: invocation.cursor,
        complete: false,
    };
    match completion.result {
        Ok(Progress::Done) => tracked.complete = true,
        Ok(Progress::Pending) => {}
        Err(error) => {
            let fail = match error {
                RunError::Step(fail) => fail,
                _ => Fail::Full,
            };
            let job = &mut tracked.operation;
            if matches!(job.request.action, Action::Ruin) && job.execution.task.is_some() {
                programs::debug::put(&alloc::format!(
                    "system: ruin {} failed {:?}",
                    job.request.name,
                    fail
                ));
                return Err(super::Fail::Shutdown);
            }
            job.failure = Some(fail);
            if matches!(job.request.action, Action::Mint | Action::Embark { .. })
                && job.execution.instance.is_some()
            {
                job.request.action = Action::Ruin;
                job.execution.deadline =
                    env::chrono::clock() + start::BOOT_MS as u64 * 1_000_000;
                tracked.cursor.reset();
            } else {
                tracked.complete = true;
            }
        }
    }
    operations.0.push_back(tracked);
    Ok(Progress::Done)
}
pub fn failed_mint(
    operations: Res<Operations>,
    mut control: ResMut<Control>,
) -> Result<Progress, super::Fail> {
    for tracked in &operations.0 {
        let job = &tracked.operation;
        if tracked.complete
            && job.failure.is_some()
            && matches!(job.request.action, Action::Mint)
            && job.execution.task.is_none()
            && control.table.find(&job.request.name).is_some_and(|row| {
                row.state == State::NeverStarted && matches!(row.slot, Slot::None)
            })
        {
            control.table.set_state(&job.request.name, State::Dead);
        }
    }
    Ok(Progress::Done)
}
