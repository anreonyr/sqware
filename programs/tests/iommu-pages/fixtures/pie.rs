use crate::{PieToken, ReleaseMode, state};

pub fn release(pole: PieToken, mode: ReleaseMode) -> abi::PieResult<()> {
    assert_eq!(pole, PieToken::mint(11));
    assert!(matches!(mode, ReleaseMode::Revoke));
    state::step("release pole").map_err(|()| abi::make_fail(abi::PieFail::Busy))?;
    state::STATE.with_borrow_mut(|state| {
        assert_eq!(state.poles, 1, "duplicate Pole release");
        assert_eq!(state.references, 0, "Pole released before reference");
        assert_eq!(state.views, 0, "Pole released before view");
        state.poles -= 1;
    });
    Ok(())
}
