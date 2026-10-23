use ::schedule::Progress;
use env::{PieToken, TeamId, UnitFail};

/// Trusted deployment guard selection. Names never enter this module.
pub(crate) fn required(guarded: &[&str], name: &str) -> bool {
    guarded.contains(&name)
}

/// Arm the team's exit guard. The kernel requires a debarked team, so pause and
/// wait first; Busy means the pause has not settled yet.
pub(crate) fn arm(team: TeamId, authority: PieToken) -> Result<Progress, UnitFail> {
    match env::unit::debark_team(team) {
        Ok(()) => {}
        Err(error) if error.source == UnitFail::Busy => return Ok(Progress::Pending),
        Err(error) => return Err(error.source),
    }
    match env::unit::guard(team, authority) {
        Ok(()) => Ok(Progress::Done),
        Err(error) if error.source == UnitFail::Busy => Ok(Progress::Pending),
        Err(error) => Err(error.source),
    }
}

/// Resume a paused team without re-arming its guard.
pub(crate) fn resume(team: TeamId) -> Result<(), UnitFail> {
    env::unit::embark_team(team).map_err(|error| error.source)
}

/// Pause the whole team until every member is off-hart, then release the guard.
/// The obligation stays held until this returns `Done`.
pub(crate) fn release(team: TeamId, authority: PieToken) -> Result<Progress, UnitFail> {
    match env::unit::debark_team(team) {
        Ok(()) => {}
        Err(error) if error.source == UnitFail::Busy => return Ok(Progress::Pending),
        Err(error) => return Err(error.source),
    }
    match env::unit::unguard(team, authority) {
        Ok(()) => Ok(Progress::Done),
        Err(error) if error.source == UnitFail::Busy => Ok(Progress::Pending),
        Err(error) => Err(error.source),
    }
}
