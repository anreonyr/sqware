use alloc::collections::VecDeque;
use ::core::cell::RefCell;
use system_api::control::{Fail, frame::Said};

#[derive(Default)]
struct Backend {
    team_complete: bool,
    team_consumed: bool,
    release_begun: usize,
    pump_steps: usize,
    release: VecDeque<Result<Option<Said>, Fail>>,
}

thread_local! {
    static BACKEND: RefCell<Backend> = RefCell::new(Backend::default());
}

pub fn reset() {
    BACKEND.with(|backend| *backend.borrow_mut() = Backend::default());
}

pub fn complete_team() {
    BACKEND.with(|backend| backend.borrow_mut().team_complete = true);
}

pub fn team_complete() -> bool {
    BACKEND.with(|backend| backend.borrow().team_complete)
}

pub fn consume_team() {
    BACKEND.with(|backend| backend.borrow_mut().team_consumed = true);
}

pub fn team_consumed() -> bool {
    BACKEND.with(|backend| backend.borrow().team_consumed)
}

pub fn release_begun() -> usize {
    BACKEND.with(|backend| backend.borrow().release_begun)
}

pub fn pump_steps() -> usize {
    BACKEND.with(|backend| backend.borrow().pump_steps)
}

pub fn pump_step() {
    BACKEND.with(|backend| backend.borrow_mut().pump_steps += 1);
}

pub fn script_release(reply: Result<Option<Said>, Fail>) {
    BACKEND.with(|backend| backend.borrow_mut().release.push_back(reply));
}

pub fn begin_release() {
    BACKEND.with(|backend| backend.borrow_mut().release_begun += 1);
}

pub fn poll_release() -> Result<Option<Said>, Fail> {
    BACKEND.with(|backend| {
        backend
            .borrow_mut()
            .release
            .pop_front()
            .unwrap_or(Ok(None))
    })
}
