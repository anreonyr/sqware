//! Native facts required for the explicitly delivered session endpoints.
use env::{PieToken, TaskId};

pub(super) fn valid_reply(who: TaskId, reply: PieToken, from: TaskId) -> bool {
    ::resource::raw::alive(reply)
        && matches!(::resource::raw::reserve(reply), Ok((vestor, owner, mark))
        if vestor == from && owner == who && mark == system_api::operator::LINK_MARK)
}

pub(super) fn valid_request(who: TaskId, ask: PieToken, from: TaskId) -> bool {
    ::resource::raw::alive(ask)
        && matches!(::resource::raw::reserve(ask), Ok((vestor, owner, mark))
        if from == who && vestor == from && owner == who && mark == system_api::operator::ASK_MARK)
}
