use crate::{PieToken, ReleaseMode, state};

pub fn release(pole: PieToken, mode: ReleaseMode) -> abi::PieResult<()> {
    assert!(matches!(mode, ReleaseMode::Revoke));
    if pole == PieToken::mint(11) {
        state::step("release pole").map_err(|()| abi::make_fail(abi::PieFail::Busy))?;
        state::STATE.with_borrow_mut(|state| {
            assert_eq!(state.poles, 1, "duplicate Pole release");
            assert_eq!(state.references, 0, "Pole released before reference");
            assert_eq!(state.views, 0, "Pole released before view");
            state.poles -= 1;
        });
        return Ok(());
    }
    state::STATE.with_borrow_mut(|state| {
        let source = state
            .sources
            .iter_mut()
            .find(|source| source.token == pole.get())
            .expect("fixture source");
        assert!(
            source.reference.is_none(),
            "source released before its data reference"
        );
        assert!(source.alive, "data source released twice");
        source.alive = false;
    });
    Ok(())
}
