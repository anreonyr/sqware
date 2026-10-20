use crate::{PieToken, state};

pub fn pole(bytes: usize, shared: bool) -> abi::PieResult<PieToken> {
    assert!(bytes > 0 && shared);
    state::step("pole").map_err(|()| abi::make_fail(abi::PieFail::Busy))?;
    state::STATE.with_borrow_mut(|state| state.poles += 1);
    Ok(PieToken::mint(11))
}
