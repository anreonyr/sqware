use crate::{Mark, Permission, PieFail, PieToken, TaskId};
use crate::state::{BAR, STATE};

pub fn accord(
    token: PieToken, _: TaskId, subset: Permission, _: Mark,
) -> Result<PieToken, PieFail> {
    assert_eq!(token, BAR);
    assert_eq!(subset, Permission::FETCH | Permission::STORE | Permission::ONLY);
    // Core measured the mapped BAR as exactly `pages * PAGE_SIZE`; Accord inherits it.
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert!(state.mapped, "recovery mapping must precede the child grant");
        if state.grant_fail {
            return Err(PieFail::OoM);
        }
        state.grants += 1;
        Ok(PieToken::mint(9))
    })
}
