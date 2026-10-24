use crate::{PieToken, state};

pub fn pole(bytes: usize, shared: bool) -> abi::PieResult<PieToken> {
    assert!(bytes > 0 && shared);
    state::step("pole").map_err(|()| abi::make_fail(abi::PieFail::Busy))?;
    state::STATE.with_borrow_mut(|state| state.poles += 1);
    Ok(PieToken::mint(11))
}

pub fn open(token: PieToken) -> abi::PoleResult<(usize, usize)> {
    match token.get() {
        1 => Ok(state::STATE.with_borrow_mut(|state| {
            if state.mmio.is_empty() { state.mmio.resize(abi::PAGE_SIZE / 8, 0); }
            (state.mmio.as_ptr() as usize, abi::PAGE_SIZE)
        })),
        42 => Ok((0, 2 * abi::PAGE_SIZE)),
        _ => Err(abi::make_fail(abi::PoleFail::Denied)),
    }
}

pub fn pies() -> impl Iterator<Item = crate::PieInfo> {
    let (alive, serial) = state::STATE.with_borrow(|s| (s.data_alive, s.window_serial));
    [crate::PieInfo {
        token: PieToken::mint(42), kind: crate::PieKind::Pole,
        permission: crate::Permission::FETCH | crate::Permission::STORE,
        owner: crate::TaskId::new(5), vestor: crate::TaskId::new(9),
        mark: hub_api::window_mark(serial), alive,
    }].into_iter()
}

pub fn alive(token: PieToken) -> bool {
    assert_eq!(token, PieToken::mint(42));
    state::STATE.with_borrow(|state| state.data_alive)
}
