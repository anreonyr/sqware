use std::cell::RefCell;

#[derive(Default)]
pub struct State {
    pub depth: usize,
    pub local: bool,
    pub remote: bool,
    pub ready_outside_queue: bool,
    pub slay_pending: bool,
    pub seated: bool,
    pub slain: bool,
}

thread_local! {
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn dequeue(remote: bool) -> Option<usize> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let queued = if remote { &mut state.remote } else { &mut state.local };
        if !*queued {
            return None;
        }
        *queued = false;
        state.ready_outside_queue = true;
        state.slay_pending = true;
        assert!(state.depth > 0, "Slay observed Ready outside its container");
        Some(1)
    })
}
