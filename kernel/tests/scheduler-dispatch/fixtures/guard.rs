use crate::state::STATE;

pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.depth -= 1;
            if state.depth == 0 && state.slay_pending {
                assert!(!state.ready_outside_queue, "Slay observed Ready outside its container");
                assert!(state.seated, "dispatch never published the running owner");
                state.slain = true;
            }
        });
    }
}
