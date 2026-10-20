use std::cell::RefCell;

#[derive(Default)]
pub struct State {
    pub failure: Option<&'static str>,
    pub events: Vec<&'static str>,
    pub poles: usize,
    pub references: usize,
    pub views: usize,
}

thread_local! {
    pub static STATE: RefCell<State> = RefCell::default();
}

pub fn step(operation: &'static str) -> Result<(), ()> {
    STATE.with_borrow_mut(|state| {
        state.events.push(operation);
        if state.failure == Some(operation) {
            Err(())
        } else {
            Ok(())
        }
    })
}

pub fn fail(operation: Option<&'static str>) {
    STATE.with_borrow_mut(|state| state.failure = operation);
}

pub fn counts() -> (usize, usize, usize) {
    STATE.with_borrow(|state| (state.poles, state.references, state.views))
}

pub fn events() -> Vec<&'static str> {
    STATE.with_borrow(|state| state.events.clone())
}
