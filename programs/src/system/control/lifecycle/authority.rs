use super::authority_verify::Verify;
use super::{Action, Active, Origin};
use crate::system::app::policy::Settle;
use crate::system::control::unit::{
    Control,
    table::{Service, Slot, State},
    verdict::Fail,
};
use crate::unit::Target;
use ::schedule::{Progress, Res};
use env::TaskId;

/// Recover admission: the actual static sender must be the named supervisor
/// for this target and the old retirement must be fully finalized.
pub fn admit(active: Res<Active>, control: Res<Control>, settle: Res<Settle>) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let Origin::Recovery { caller } = job.key.request.directive.origin else {
        return Ok(Progress::Done);
    };
    if !matches!(job.key.request.directive.action, Action::Recover) {
        return Err(Fail::Denied);
    }
    Verify {
        control: &control,
        settle: &settle,
        caller,
        target: &job.key.request.name,
    }
    .run(true)?;
    Ok(Progress::Done)
}

/// Effect recheck: repeat caller scope, target intent and global closing.
/// It deliberately does not re-demand the retire-complete slot after a
/// recovery Mint has attached the new service.
pub fn recheck(
    active: Res<Active>,
    control: Res<Control>,
    settle: Res<Settle>,
) -> Result<Progress, Fail> {
    verify_effect(&active, &control, &settle)?;
    Ok(Progress::Done)
}

pub(crate) fn verify_effect(
    active: &Active,
    control: &Control,
    settle: &Settle,
) -> Result<(), Fail> {
    let Some(job) = active.0.as_ref() else {
        return Err(Fail::Unknown);
    };
    let Origin::Recovery { caller } = job.key.request.directive.origin else {
        return Ok(());
    };
    if !matches!(
        job.key.request.directive.action,
        Action::Recover | Action::Embark { parent: None }
    ) {
        return Err(Fail::Denied);
    }
    Verify {
        control,
        settle,
        caller,
        target: &job.key.request.name,
    }
    .run(false)
}

pub(crate) fn supervisor(control: &Control, caller: TaskId) -> Result<&Service, Fail> {
    control
        .table
        .rows()
        .find(|row| {
            matches!(row.slot, Slot::Live { task, .. } if task == caller)
                && row.named
                && row.run_requested
                && row.state == State::Ready
                && control.live(caller)
        })
        .ok_or(Fail::Denied)
}

pub(crate) fn targets(control: &Control, caller: TaskId) -> Result<&'static Target, Fail> {
    let source = supervisor(control, caller)?;
    group(control, source.name.as_str())
}

pub(crate) fn group(control: &Control, name: &str) -> Result<&'static Target, Fail> {
    let group = control
        .input(name)?
        .program
        .control
        .supervision
        .ok_or(Fail::Denied)?;
    control
        .targets
        .iter()
        .find(|target| target.name == group)
        .ok_or(Fail::Denied)
}
