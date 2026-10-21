use std::cell::RefCell;
use std::collections::VecDeque;

use crate::system_api;

#[derive(Default)]
struct Backend {
    grant: bool,
    discover: VecDeque<Result<(), ()>>,
    begin: VecDeque<Result<(), ()>>,
    poll: VecDeque<Result<Option<system_api::retirement::Retirement>, ()>>,
    discover_calls: usize,
    begin_calls: usize,
    poll_calls: usize,
    targets: Vec<crate::TaskId>,
}

thread_local! {
    static BACKEND: RefCell<Backend> = RefCell::new(Backend { grant: true, ..Backend::default() });
}

pub(crate) fn reset(
    discover: Vec<Result<(), ()>>,
    begin: Vec<Result<(), ()>>,
    poll: Vec<Result<Option<system_api::retirement::Retirement>, ()>>,
) {
    BACKEND.with(|backend| {
        *backend.borrow_mut() = Backend {
            grant: true,
            discover: VecDeque::from(discover),
            begin: VecDeque::from(begin),
            poll: VecDeque::from(poll),
            discover_calls: 0,
            begin_calls: 0,
            poll_calls: 0,
            targets: Vec::new(),
        };
    });
}

pub(crate) fn deny_grant() {
    BACKEND.with(|backend| backend.borrow_mut().grant = false);
}

pub(crate) fn grant() -> bool {
    BACKEND.with(|backend| backend.borrow().grant)
}

pub(crate) fn discover() -> Result<(), ()> {
    BACKEND.with(|backend| {
        let mut backend = backend.borrow_mut();
        backend.discover_calls += 1;
        backend.discover.pop_front().unwrap_or(Err(()))
    })
}

pub(crate) fn begin(target: crate::TaskId) -> Result<(), ()> {
    BACKEND.with(|backend| {
        let mut backend = backend.borrow_mut();
        backend.begin_calls += 1;
        backend.targets.push(target);
        backend.begin.pop_front().unwrap_or(Err(()))
    })
}

pub(crate) fn poll() -> Result<Option<system_api::retirement::Retirement>, ()> {
    BACKEND.with(|backend| {
        let mut backend = backend.borrow_mut();
        backend.poll_calls += 1;
        backend.poll.pop_front().unwrap_or(Err(()))
    })
}

pub(crate) fn calls() -> (usize, usize, usize) {
    BACKEND.with(|backend| {
        let backend = backend.borrow();
        (backend.discover_calls, backend.begin_calls, backend.poll_calls)
    })
}

pub(crate) fn targets() -> Vec<crate::TaskId> {
    BACKEND.with(|backend| backend.borrow().targets.clone())
}
