use crate::{PieResult, PieToken, PoleResult, TaskId};

pub use crate::capability::Capability;

pub fn open(token: PieToken) -> PoleResult<(usize, usize)> {
    use crate::PoleFail;
    if token == PieToken::NONE {
        return Err(crate::make_fail(PoleFail::Denied));
    }
    Ok((0, 0))
}

pub fn release(token: PieToken) -> PieResult<()> {
    crate::state::release(token)
}

pub fn revoke(peer: TaskId, token: PieToken) -> PieResult<()> {
    crate::state::revoke(peer, token);
    Ok(())
}
