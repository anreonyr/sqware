extern crate self as env;

pub use abi::{
    HoleDir, MailFail, MailResult, Mark, Permission, PieFail, PieResult, PieToken, TaskId,
    VirtAddr, Wait, make_fail,
};

pub mod chrono {
    pub fn clock() -> u64 { 0 }
}

pub mod pie {
    use crate::{
        Mark, Permission, PieFail, PieResult, PieToken, TaskId, VirtAddr, make_fail,
        test_backend::{Event, Op, record, token},
    };

    pub fn unseal_hole(mark: Mark) -> PieResult<PieToken> {
        let token = token();
        record(Event::Unseal(token, mark));
        if crate::test_backend::fail(Op::Unseal) { Err(make_fail(PieFail::Denied)) } else { Ok(token) }
    }

    pub fn release(token: PieToken) -> PieResult<()> {
        record(Event::Release(token));
        if crate::test_backend::fail(Op::Release) { Err(make_fail(PieFail::Denied)) } else { Ok(()) }
    }

    pub fn seal(token: PieToken) -> PieResult<()> {
        record(Event::Seal(token));
        if crate::test_backend::fail(Op::Seal) { Err(make_fail(PieFail::Denied)) } else { Ok(()) }
    }

    pub fn accord(src: PieToken, peer: TaskId, permission: Permission, mark: Mark) -> PieResult<PieToken> {
        let remote = token();
        record(Event::Accord(src, peer, permission, mark, remote));
        if crate::test_backend::fail(Op::Accord) { Err(make_fail(PieFail::Denied)) } else { Ok(remote) }
    }

    pub fn revoke(peer: TaskId, remote: PieToken) -> PieResult<()> {
        record(Event::Revoke(peer, remote));
        if crate::test_backend::fail(Op::Revoke) { Err(make_fail(PieFail::Denied)) } else { Ok(()) }
    }

    pub fn reserve(_: PieToken) -> PieResult<(usize, usize)> { Ok((0, 0)) }
    pub fn inspect(_: PieToken) -> PieResult<(usize, usize)> { Ok((0, 0)) }
    pub fn alive(_: PieToken) -> PieResult<bool> { Ok(true) }
    pub fn collect(_: usize) -> (PieToken, TaskId, Mark) {
        (PieToken::NONE, TaskId::new(0), Mark::NONE)
    }
    pub fn open(_: PieToken) -> PieResult<(VirtAddr, usize)> { Ok((VirtAddr::new(0), 0)) }
}

pub mod mail {
    use crate::{
        HoleDir, MailFail, MailResult, PieToken, TaskId, VirtAddr, Wait, make_fail,
        test_backend::{Event, pop_message, record},
    };

    pub fn pull(token: PieToken, buf: VirtAddr, max: usize) -> MailResult<(usize, TaskId)> {
        record(Event::Pull(token));
        let (bytes, from) = pop_message().ok_or_else(|| make_fail(MailFail::Busy))?;
        if bytes.len() > max { return Err(make_fail(MailFail::Denied)); }
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.get() as *mut u8, bytes.len()); }
        Ok((bytes.len(), from))
    }

    pub fn push(_: PieToken, _: VirtAddr, _: usize) -> MailResult<()> { Ok(()) }
    pub fn peek(_: PieToken) -> MailResult<(usize, TaskId, usize)> { Err(make_fail(MailFail::Busy)) }
    pub fn wait(_: PieToken, _: HoleDir, _: Wait) -> MailResult<bool> { Ok(false) }
    pub fn ring(_: PieToken) -> MailResult<()> { Ok(()) }
    pub fn hush(_: PieToken) -> MailResult<()> { Ok(()) }
}

pub mod test_backend {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use abi::{Mark, Permission, PieToken, TaskId};

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Op { Unseal, Release, Seal, Accord, Revoke }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Event {
        Unseal(PieToken, Mark),
        Release(PieToken),
        Seal(PieToken),
        Accord(PieToken, TaskId, Permission, Mark, PieToken),
        Revoke(TaskId, PieToken),
        Pull(PieToken),
    }

    #[derive(Default)]
    struct State {
        next: usize,
        events: Vec<Event>,
        fail_next: Option<Op>,
        messages: VecDeque<(Vec<u8>, TaskId)>,
    }

    thread_local! { static STATE: RefCell<State> = RefCell::new(State { next: 100, ..State::default() }); }

    pub fn reset() { STATE.with(|s| *s.borrow_mut() = State { next: 100, ..State::default() }); }
    pub fn events() -> Vec<Event> { STATE.with(|s| s.borrow().events.clone()) }
    pub fn fail_next(op: Op) { STATE.with(|s| s.borrow_mut().fail_next = Some(op)); }
    pub fn queue_message(bytes: &[u8], from: TaskId) {
        STATE.with(|s| s.borrow_mut().messages.push_back((bytes.to_vec(), from)));
    }
    pub fn token() -> PieToken {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let token = PieToken::mint(state.next);
            state.next += 1;
            token
        })
    }
    pub fn record(event: Event) { STATE.with(|s| s.borrow_mut().events.push(event)); }
    pub fn fail(op: Op) -> bool {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if state.fail_next == Some(op) {
                state.fail_next = None;
                true
            } else { false }
        })
    }
    pub fn pop_message() -> Option<(Vec<u8>, TaskId)> {
        STATE.with(|s| s.borrow_mut().messages.pop_front())
    }
}

#[path = "../../src/raw.rs"]
pub mod raw;
#[path = "../../src/hole.rs"]
#[allow(dead_code)]
pub(crate) mod hole;
#[path = "../../src/capability.rs"]
pub mod capability;
#[path = "../../src/reply.rs"]
pub mod reply;

#[cfg(test)]
mod tests;
