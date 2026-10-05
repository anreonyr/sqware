use super::answer::Inbox;
use crate::system::control::serve::Fail;
use crate::system::run::watch::Interests;
use runtime::schedule::{Progress, Res, ResMut};
pub fn entries(inbox: Res<Inbox>, mut wanted: ResMut<Interests>) -> Result<Progress, Fail> {
    wanted.tokens.extend(inbox.entry);
    Ok(Progress::Done)
}
