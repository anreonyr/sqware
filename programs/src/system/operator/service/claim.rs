//! Operator bootstrap imports explicit replies and discovers only unique request roles.
use env::{PieToken, TaskId};
use ipc::session::establish::{self, DiscoveryFail};

pub(super) fn valid_reply(who: TaskId, reply: PieToken, from: TaskId) -> bool {
    ::resource::raw::alive(reply)
        && matches!(::resource::raw::reserve(reply), Ok((vestor, owner, mark))
        if vestor == from && owner == who && mark == system_api::operator::LINK_MARK)
}

pub(super) fn ask_of(who: TaskId) -> Result<PieToken, DiscoveryFail> {
    establish::find(who, system_api::operator::ASK_MARK)
}
