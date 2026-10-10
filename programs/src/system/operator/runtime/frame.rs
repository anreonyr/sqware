use super::Fail;
use crate::system::app::life::{Phase, Status};
use ::schedule::{Progress, Res, ResMut};
use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use env::{TaskId, Wait};
use system_client::identity::TaskQuery;
pub(super) struct Running(pub bool);
pub(super) fn health(
    status: Res<Arc<Status>>,
    mut running: ResMut<Running>,
) -> Result<Progress, Fail> {
    if status.phase.load(Ordering::Acquire) == Phase::Stopping as u8 {
        running.0 = false;
        return Ok(Progress::Pending);
    }
    if env::unit::join_task(status.control, Wait::POLL).unwrap_or(true)
        || env::unit::join_task(
            TaskId::new(status.identity.load(Ordering::Acquire)),
            Wait::POLL,
        )
        .unwrap_or(true)
    {
        return Err(Fail::Dead);
    }
    Ok(Progress::Done)
}

pub(super) fn query(mut query: ResMut<Option<TaskQuery>>) -> Result<Progress, Fail> {
    if query.as_ref().is_some_and(|bundle| !bundle.available()) {
        *query = None;
    }
    Ok(Progress::Done)
}
