use super::answer::Inbox;
use crate::system::app::Fault as Fail;
use crate::system::app::wait::Waiting;

use ::schedule::{Progress, Res, ResMut};

pub(super) fn settle(
    flow: Res<crate::system::app::policy::Flow>,
    mut requests: ResMut<crate::system::launch::Requests>,
) -> Result<Progress, Fail> {
    if flow.settling {
        for request in requests.0.drain(..) {
            crate::system::loader::release_image(&request.ask, request.from);
            crate::system::launch::reply(request.delivery.back, Err(system_api::control::Fail::NotReady));
        }
    }
    Ok(Progress::Done)
}
pub(super) fn build(mut loader: ResMut<crate::system::loader::Loader>, mut pending: ResMut<crate::system::launch::Pending>, mut requests: ResMut<crate::system::launch::Requests>) -> Result<Progress, Fail> {
    for request in requests.0.drain(..) { crate::system::launch::construct(&mut loader, &mut pending, request); }
    Ok(Progress::Done)
}
pub(super) fn close(
    mut inbox: ResMut<Inbox>,
    mut loader: ResMut<crate::system::loader::Loader>,
    waiting: Res<Waiting>,
) -> Result<Progress, Fail> {
    if let Some(entry) = inbox.entry {
        waiting.detach(entry);
        env::pie::seal(entry).map_err(|_| Fail::Shutdown)?;
        env::pie::release(entry, env::ReleaseMode::Revoke).map_err(|_| Fail::Shutdown)?;
        inbox.entry = None;
    }
    loader.clear_images();
    Ok(Progress::Done)
}
