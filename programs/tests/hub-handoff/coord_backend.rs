use core::cell::RefCell;
use hub_api::{DmaSaid, coordination::Wire};
use system_api::control::Fail;

mod state;
use state::State;

thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

pub fn reset() { STATE.with(|state| *state.borrow_mut() = State::default()); }

pub fn begin(request: Wire) { STATE.with(|state| state.borrow_mut().began.push(request)); }

pub fn requests() -> Vec<Wire> { STATE.with(|state| state.borrow().began.clone()) }

pub fn script(serial: u64, result: Result<Option<DmaSaid>, Fail>) {
    STATE.with(|state| state.borrow_mut().replies.push_back((serial, result)));
}

pub fn poll(serial: u64) -> Result<Option<DmaSaid>, Fail> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(at) = state.replies.iter().position(|(id, _)| *id == serial) else { return Ok(None); };
        state.replies.remove(at).unwrap().1
    })
}
