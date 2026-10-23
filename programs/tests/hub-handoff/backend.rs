use ::core::cell::RefCell;
use alloc::collections::VecDeque;
use system_api::control::{Fail, frame::Said};

mod kind;
pub use kind::Kind;

#[derive(Default)]
struct Backend {
    inspect: VecDeque<Result<Option<Said>, Fail>>,
    embark: VecDeque<Result<Option<Said>, Fail>>,
    release: VecDeque<Result<Option<Said>, Fail>>,
    began: alloc::vec::Vec<Kind>,
}

thread_local! {
    static BACKEND: RefCell<Backend> = RefCell::new(Backend::default());
}

pub fn reset() {
    BACKEND.with(|backend| *backend.borrow_mut() = Backend::default());
}

pub fn script_inspect(reply: Result<Option<Said>, Fail>) {
    BACKEND.with(|backend| backend.borrow_mut().inspect.push_back(reply));
}

pub fn script_embark(reply: Result<Option<Said>, Fail>) {
    BACKEND.with(|backend| backend.borrow_mut().embark.push_back(reply));
}

pub fn script_release(reply: Result<Option<Said>, Fail>) {
    BACKEND.with(|backend| backend.borrow_mut().release.push_back(reply));
}

pub fn began(kind: Kind) -> usize {
    BACKEND.with(|backend| backend.borrow().began.iter().filter(|seen| **seen == kind).count())
}

pub fn begin(kind: Kind) {
    BACKEND.with(|backend| backend.borrow_mut().began.push(kind));
}

pub fn poll(kind: Kind) -> Result<Option<Said>, Fail> {
    BACKEND.with(|backend| {
        let mut backend = backend.borrow_mut();
        let queue = match kind {
            Kind::Inspect => &mut backend.inspect,
            Kind::Embark => &mut backend.embark,
            Kind::Release => &mut backend.release,
        };
        queue.pop_front().unwrap_or(Ok(None))
    })
}

pub fn reply(said: Said) -> Result<(), Fail> {
    match system_api::control::frame::code_to_fail(said.status) {
        None if said.status == system_api::control::frame::OK => Ok(()),
        Some(fail) => Err(fail),
        None => Err(Fail::Bad),
    }
}
