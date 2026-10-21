use crate::{PieInfo, PieToken, PoleResult};
use crate::state::{BAR, STATE};

pub fn inspect(token: PieToken) -> Result<PieInfo, ()> {
    assert_eq!(token, BAR);
    Ok(STATE.with(|state| state.borrow().info))
}

pub fn open(token: PieToken) -> PoleResult<(usize, usize)> {
    assert_eq!(token, BAR);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.opens += 1;
        state.mapped = true;
        Ok((state.memory.as_ptr() as usize, state.memory.len() * 8))
    })
}
