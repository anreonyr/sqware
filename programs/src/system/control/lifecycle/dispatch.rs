use super::{Action, Active, Key, Operations, Tracked};
use crate::system::control::{Fail as ControlFail, core::verdict::Fail};
use ::schedule::{Dispatch, Invocation, Progress, Res, ResMut, RunError};

pub fn budget(
    operations: Res<Operations>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, ControlFail> {
    dispatch
        .begin(operations.0.len())
        .map_err(|_| ControlFail::Room)?;
    Ok(Progress::Done)
}
pub fn select(
    mut operations: ResMut<Operations>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, ControlFail> {
    let tracked = loop {
        let Some(tracked) = operations.0.pop_front() else {
            dispatch.stop().map_err(|_| ControlFail::Room)?;
            return Ok(Progress::Done);
        };
        if !tracked.complete {
            break tracked;
        }
        operations.0.push_back(tracked);
        dispatch.skip().map_err(|_| ControlFail::Room)?;
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
    dispatch
        .select(Invocation {
            key,
            cursor: tracked.cursor,
        })
        .map_err(|_| ControlFail::Room)?;
    Ok(Progress::Done)
}
pub fn finish(
    mut operations: ResMut<Operations>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, Fail>>,
) -> Result<Progress, ControlFail> {
    let completion = dispatch.take_result().map_err(|_| ControlFail::Room)?;
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
    operations.0.push_back(tracked);
    Ok(Progress::Done)
}
