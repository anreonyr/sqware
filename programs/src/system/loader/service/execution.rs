use super::namespace::EndpointState;
use crate::system::app::Fault as Fail;
use crate::system::app::wait::Waiting;

use ::schedule::{Progress, Res, ResMut};

pub(super) fn close(
    mut endpoints: ResMut<EndpointState>,
    mut loader: ResMut<crate::system::loader::Loader>,
    waiting: Res<Waiting>,
) -> Result<Progress, Fail> {
    endpoints.retire_claims().map_err(|_| Fail::Shutdown)?;
    let inbox = &mut endpoints.inbox;
    if let Some(entry) = inbox.entry {
        waiting.detach(entry);
        env::hole::seal(entry).map_err(|_| Fail::Shutdown)?;
        env::pie::release(entry, env::ReleaseMode::Revoke).map_err(|_| Fail::Shutdown)?;
        inbox.entry = None;
    }
    loader.clear_images();
    Ok(Progress::Done)
}
