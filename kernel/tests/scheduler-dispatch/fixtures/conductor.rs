use crate::state::{STATE, dequeue};

pub struct Conductor;

impl Conductor {
    pub fn pull(&self) -> Option<usize> {
        dequeue(false)
    }

    pub fn seat(&self, task: usize) -> Option<usize> {
        assert_eq!(task, 1);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            assert!(state.depth > 0, "seat escaped the dequeue commit");
            state.ready_outside_queue = false;
            state.seated = true;
        });
        Some(0x1000)
    }
}
