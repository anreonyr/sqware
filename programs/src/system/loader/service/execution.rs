use super::answer::Inbox;
use crate::system::app::Fault as Fail;
use crate::system::app::wait::Waiting;

use ::schedule::{Progress, Res, ResMut};

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
