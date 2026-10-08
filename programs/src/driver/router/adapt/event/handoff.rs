use ::resource::raw::{alive, reserve};
use env::{Mark, PieToken, TaskId};
use ipc::session::{Held, establish};

pub(super) fn lane(from: TaskId, token: PieToken) -> Option<Held> {
    trusted(from, token, router_api::LINE_MARK)?;
    let lane = Held(establish::accept(token).ok()?);
    lane.tx()?;
    Some(lane)
}

pub(super) fn back(from: TaskId, token: PieToken) -> Option<PieToken> {
    trusted(from, token, router_api::frame::BACK_MARK).map(|_| token)
}

fn trusted(from: TaskId, token: PieToken, expected: Mark) -> Option<()> {
    if !alive(token) {
        return None;
    }
    matches!(reserve(token), Ok((vestor, owner, mark))
        if vestor == from && owner == from && mark == expected)
    .then_some(())
}
