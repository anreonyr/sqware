use crate::{PieToken, PoleFail, PoleResult};
use crate::state::{BAR, STATE};

pub fn shut(token: PieToken) -> PoleResult<()> {
    assert_eq!(token, BAR);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert!(state.mapped, "must retain the actual mapped handle");
        state.shuts += 1;
        if state.shut_fail {
            return Err(PoleFail::Busy);
        }
        state.mapped = false;
        Ok(())
    })
}
