use super::{Action, Active, Key, Tracked};
use crate::system::app::Fault as ControlFail;
use crate::system::control::unit::verdict::Fail;

use ::schedule::{Dispatch, Invocation, Progress, Res, ResMut, RunError};

pub fn budget(
    operations: Res<crate::system::assembly::Jobs>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, ControlFail> {
    dispatch.begin(operations.len())?;
    Ok(Progress::Done)
}
pub fn select(
    mut operations: ResMut<crate::system::assembly::Jobs>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, ControlFail> {
    let tracked = loop {
        let Some(tracked) = operations.select_lifecycle() else {
            dispatch.stop()?;
            return Ok(Progress::Done);
        };
        break tracked;
    };
    let key = match tracked.operation.key.request.directive.action {
        Action::Mint | Action::Recover => Key::Mint,
        Action::Embark { .. } => Key::Embark,
        Action::Debark => Key::Debark,
        Action::Ruin => Key::Ruin,
    };
    active.0 = Some(tracked.operation);
    dispatch.select(Invocation {
        key,
        cursor: tracked.cursor,
    })?;
    Ok(Progress::Done)
}
pub fn finish(
    mut operations: ResMut<crate::system::assembly::Jobs>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, ControlFail> {
    let completion = dispatch.take_result()?;
    let invocation = completion.invocation;
    let mut tracked = Tracked {
        operation: active.0.take().ok_or(ControlFail::Room)?,
        cursor: invocation.cursor,
        complete: false,
    };
    let result = completion.result.map_err(|error| match error {
        RunError::Step(fail) => fail,
        _ => Fail::Full,
    });
    tracked.finish(result)?;
    if tracked.complete {
        operations.complete_lifecycle(tracked);
    } else {
        operations.requeue_lifecycle(tracked);
    }
    Ok(Progress::Done)
}
