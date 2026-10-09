extern crate self as env;

pub use abi::{
    MailCondition, MailFail, MailResult, Mark, Oversize, Permission, PieFail, PieInfo, PieKind,
    PieResult, PieToken, PullOutcome, ReleaseMode, TaskId, UnsealArgs, VirtAddr, Wait, make_fail,
};

pub mod chrono {
    pub fn clock() -> u64 {
        crate::test_backend::now()
    }
}
pub mod room {
    pub fn starve() {
        crate::test_backend::record(crate::test_backend::Event::Starve);
    }
}

pub mod pie {
    pub fn unseal(args: crate::UnsealArgs) -> PieResult<PieToken> {
        match args {
            crate::UnsealArgs::Hole { mark, .. } => hole_token(mark),
            _ => unreachable!(),
        }
    }

    use crate::{
        Mark, Permission, PieFail, PieResult, PieToken, TaskId, VirtAddr, make_fail,
        test_backend::{Event, Op, record, token},
    };

    fn hole_token(mark: Mark) -> PieResult<PieToken> {
        let token = token();
        record(Event::Unseal(token, mark));
        if crate::test_backend::fail(Op::Unseal) {
            Err(make_fail(PieFail::Denied))
        } else {
            Ok(token)
        }
    }

    pub fn release(token: PieToken, _mode: crate::ReleaseMode) -> PieResult<()> {
        record(Event::Release(token));
        if crate::test_backend::busy(Op::Release) {
            Err(make_fail(PieFail::Busy))
        } else if crate::test_backend::fail(Op::Release) {
            Err(make_fail(PieFail::Denied))
        } else {
            Ok(())
        }
    }

    pub fn seal(token: PieToken) -> PieResult<()> {
        record(Event::Seal(token));
        if crate::test_backend::fail(Op::Seal) {
            Err(make_fail(PieFail::Denied))
        } else {
            Ok(())
        }
    }

    pub fn accord(
        src: PieToken,
        peer: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> PieResult<PieToken> {
        let remote = token();
        record(Event::Accord(src, peer, permission, mark, remote));
        if crate::test_backend::fail(Op::Accord) {
            Err(make_fail(PieFail::Denied))
        } else {
            Ok(remote)
        }
    }

    pub fn revoke(peer: TaskId, remote: PieToken) -> PieResult<()> {
        record(Event::Revoke(peer, remote));
        if crate::test_backend::busy(Op::Revoke) {
            Err(make_fail(PieFail::Busy))
        } else if crate::test_backend::fail(Op::Revoke) {
            Err(make_fail(PieFail::Denied))
        } else {
            Ok(())
        }
    }

    pub fn inspect(token: PieToken, buf: VirtAddr) -> PieResult<()> {
        let words = crate::PieInfo {
            token,
            kind: crate::PieKind::Hole,
            permission: Permission::FETCH | Permission::STORE | Permission::VEST,
            owner: TaskId::new(0),
            vestor: TaskId::new(0),
            mark: Mark::NONE,
            alive: true,
        }
        .words();
        unsafe {
            core::ptr::copy_nonoverlapping(words.as_ptr(), buf.get() as *mut usize, words.len());
        }
        Ok(())
    }
    pub fn collect(_: PieToken, _: VirtAddr, _: usize) -> PieResult<usize> {
        Ok(0)
    }
    pub fn open(_: PieToken) -> PieResult<(VirtAddr, usize)> {
        Ok((VirtAddr::new(0), 0))
    }
}

pub mod mail {
    use crate::{
        MailCondition, MailFail, MailResult, PieToken, TaskId, VirtAddr, Wait, make_fail,
        test_backend::{Event, pop_message, record},
    };

    pub fn pull(
        token: PieToken,
        buf: VirtAddr,
        max: usize,
        oversize: crate::Oversize,
    ) -> MailResult<crate::PullOutcome> {
        record(Event::Pull(token));
        let (bytes, from) = pop_message().ok_or_else(|| make_fail(MailFail::Busy))?;
        if bytes.len() > max {
            return if oversize == crate::Oversize::Discard {
                Ok(crate::PullOutcome::Discarded {
                    len: bytes.len(),
                    sender: from,
                })
            } else {
                crate::test_backend::return_message(bytes, from);
                Err(make_fail(MailFail::Denied))
            };
        }
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.get() as *mut u8, bytes.len());
        }
        Ok(crate::PullOutcome::Received {
            len: bytes.len(),
            sender: from,
        })
    }

    pub fn push(_: PieToken, _: VirtAddr, _: usize) -> MailResult<()> {
        Ok(())
    }
    pub fn peek(_: PieToken) -> MailResult<(usize, TaskId, usize)> {
        Err(make_fail(MailFail::Busy))
    }
    pub fn wait(token: PieToken, condition: MailCondition, within: Wait) -> MailResult<bool> {
        record(Event::Wait(token, condition, within));
        if let Wait::AtMost(ms) = within { crate::test_backend::advance(ms as u64 * 1_000_000); }
        Ok(false)
    }
    pub fn ring(_: PieToken) -> MailResult<()> {
        Ok(())
    }
    pub fn hush(_: PieToken) -> MailResult<()> {
        Ok(())
    }
}

pub mod test_backend {
    use abi::{MailCondition, Mark, Permission, PieToken, TaskId, Wait};
    use std::cell::RefCell;
    use std::collections::VecDeque;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Op {
        Unseal,
        Release,
        Seal,
        Accord,
        Revoke,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Event {
        Starve,
        Unseal(PieToken, Mark),
        Release(PieToken),
        Seal(PieToken),
        Accord(PieToken, TaskId, Permission, Mark, PieToken),
        Revoke(TaskId, PieToken),
        Pull(PieToken),
        Wait(PieToken, MailCondition, Wait),
    }

    #[derive(Default)]
    struct State {
        next: usize,
        now: u64,
        events: Vec<Event>,
        fail_next: Option<Op>,
        busy: Option<(Op, usize)>,
        messages: VecDeque<(Vec<u8>, TaskId)>,
    }

    thread_local! { static STATE: RefCell<State> = RefCell::new(State { next: 100, ..State::default() }); }

    pub fn reset() {
        STATE.with(|s| {
            *s.borrow_mut() = State {
                next: 100,
                ..State::default()
            }
        });
    }
    pub fn events() -> Vec<Event> {
        STATE.with(|s| s.borrow().events.clone())
    }
    pub fn fail_next(op: Op) {
        STATE.with(|s| s.borrow_mut().fail_next = Some(op));
    }
    pub fn busy_for(op: Op, count: usize) {
        STATE.with(|s| s.borrow_mut().busy = Some((op, count)));
    }
    pub fn busy(op: Op) -> bool {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            match &mut state.busy {
                Some((operation, count)) if *operation == op && *count > 0 => {
                    *count -= 1;
                    true
                }
                _ => false,
            }
        })
    }
    pub fn queue_message(bytes: &[u8], from: TaskId) {
        STATE.with(|s| s.borrow_mut().messages.push_back((bytes.to_vec(), from)));
    }

    pub fn now() -> u64 { STATE.with(|state| state.borrow().now) }
    pub fn advance(ns: u64) { STATE.with(|state| state.borrow_mut().now += ns); }
    pub fn token() -> PieToken {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let token = PieToken::mint(state.next);
            state.next += 1;
            token
        })
    }
    pub fn record(event: Event) {
        STATE.with(|s| s.borrow_mut().events.push(event));
    }
    pub fn fail(op: Op) -> bool {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if state.fail_next == Some(op) {
                state.fail_next = None;
                true
            } else {
                false
            }
        })
    }
    pub fn return_message(bytes: Vec<u8>, from: TaskId) {
        STATE.with(|s| s.borrow_mut().messages.push_front((bytes, from)));
    }
    pub fn pop_message() -> Option<(Vec<u8>, TaskId)> {
        STATE.with(|s| s.borrow_mut().messages.pop_front())
    }
}

#[path = "../../src/capability.rs"]
pub mod capability;
#[path = "../../src/hole.rs"]
#[allow(dead_code)]
pub(crate) mod hole;
#[path = "../../src/raw.rs"]
pub mod raw;
#[path = "../../src/reply.rs"]
pub mod reply;

#[cfg(test)]
mod tests;
