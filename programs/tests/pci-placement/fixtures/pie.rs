use crate::{GrantRange, Mark, Permission, PieResult, PieToken, ReleaseMode, TaskId, state};

pub fn derive(
    source: PieToken,
    permission: Permission,
    mark: Mark,
    range: GrantRange,
) -> PieResult<PieToken> {
    state::derive(source, permission, mark, range)
}

pub fn accord(
    source: PieToken,
    peer: TaskId,
    permission: Permission,
    mark: Mark,
) -> PieResult<PieToken> {
    state::accord(source, peer, permission, mark)
}

pub fn release(token: PieToken, _: ReleaseMode) -> PieResult<()> {
    state::release(token)
}

pub fn revoke(peer: TaskId, token: PieToken) -> PieResult<()> {
    state::revoke(peer, token);
    Ok(())
}
