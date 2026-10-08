use ::resource::raw::{alive, reserve};
use env::{PieToken, TaskId};

pub(super) fn valid_source(source: Option<TaskId>, host: TaskId) -> bool {
    source == Some(host)
}

pub(super) fn valid_lane(host: TaskId, token: PieToken) -> bool {
    token != PieToken::NONE
        && alive(token)
        && matches!(reserve(token), Ok((vestor, owner, mark))
            if vestor == host && owner == host && mark == router_api::LINE_MARK)
}
