use super::answer::Inbox;
use crate::system::app::Fault as Fail;
use crate::system::app::wait::Interests;
use ::schedule::{Progress, Res, ResMut};
pub(crate) fn entries(inbox: Res<Inbox>, mut wanted: ResMut<Interests>) -> Result<Progress, Fail> {
    wanted.tokens.extend(inbox.entry);
    Ok(Progress::Done)
}
