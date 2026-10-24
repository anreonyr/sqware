use crate::{PieInfo, PieKind, PieToken, Permission, TaskId, state};

pub fn pole(bytes: usize, shared: bool) -> abi::PieResult<PieToken> {
    assert!(bytes > 0 && shared);
    state::step("pole").map_err(|()| abi::make_fail(abi::PieFail::Busy))?;
    state::STATE.with_borrow_mut(|state| state.poles += 1);
    Ok(PieToken::mint(11))
}

pub fn open(token: PieToken) -> abi::PoleResult<(usize, usize)> {
    if token == PieToken::mint(1) {
        return Ok(state::STATE.with_borrow_mut(|state| {
            if state.mmio.is_empty() {
                state.mmio.resize(abi::PAGE_SIZE / 8, 0);
            }
            (state.mmio.as_ptr() as usize, abi::PAGE_SIZE)
        }));
    }
    match state::window_source(token) {
        Some(bytes) => Ok((0x1000_0000 + token.get() * 0x10_000, bytes)),
        None => Err(abi::make_fail(abi::PoleFail::Denied)),
    }
}

pub fn inspect(token: PieToken) -> abi::PieResult<PieInfo> {
    state::source(token.get())
        .map(|source| PieInfo {
            token,
            kind: source.kind(),
            permission: source.permission,
            owner: TaskId::new(5),
            vestor: source.vestor,
            mark: source.mark,
            alive: source.alive,
        })
        .ok_or(abi::make_fail(abi::PieFail::Denied))
}

pub fn pies() -> impl Iterator<Item = PieInfo> {
    let infos: Vec<PieInfo> = state::STATE.with_borrow(|state| {
        state
            .sources
            .iter()
            .map(|source| PieInfo {
                token: PieToken::mint(source.token),
                kind: PieKind::Pole,
                permission: Permission::FETCH | Permission::STORE,
                owner: TaskId::new(5),
                vestor: source.vestor,
                mark: source.mark,
                alive: source.alive,
            })
            .collect()
    });
    infos.into_iter()
}

pub fn alive(token: PieToken) -> bool {
    inspect(token).is_ok_and(|info| info.alive)
}
