use env::{JoinReply, TaskId, TeamId, UnitFail, UnitTarget, Wait};

mod error;
pub use error::Error;

pub fn self_id() -> TaskId {
    TaskId::new(1)
}

pub fn join(
    _: UnitTarget,
    _: Wait,
    receive: bool,
) -> Result<JoinReply, Error> {
    if crate::backend::team_consumed() {
        return Err(Error { source: UnitFail::Denied });
    }
    if receive {
        return Ok(JoinReply::Pending);
    }
    if crate::backend::team_complete() {
        Ok(JoinReply::Complete)
    } else {
        Ok(JoinReply::Pending)
    }
}

pub fn slay_team(_: TeamId) -> Result<(), Error> {
    Ok(())
}

pub fn embark_task(_: TaskId) -> Result<(), Error> {
    Ok(())
}

pub fn embark_team(_: TeamId) -> Result<(), Error> {
    Ok(())
}

pub fn debark_team(_: TeamId) -> Result<(), Error> {
    Ok(())
}
