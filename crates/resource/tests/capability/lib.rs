extern crate self as env;

pub use abi::{
    AcquireSource, Allocation, Bit, FlatReference, GrantRange, HoleCondition, HoleFail, HoleResult, HoleLimits, Mark,
    MemoryFail, MemoryResult, Oversize, PAGE_SIZE, Permission, PieFail, PieInfo, PieKind,
    PieResult, PieToken, PoleFail, PoleResult, PullOutcome, ReleaseMode, ToleEventSource, TaskId, VirtAddr, Wait,
    make_fail,
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
    use crate::{
        Mark, Permission, PieFail, PieResult, PieToken, TaskId, VirtAddr, make_fail,
        test_backend::{Event, Op, record, token},
    };

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

    pub fn derive(
        src: PieToken,
        permission: Permission,
        mark: Mark,
        range: crate::GrantRange,
    ) -> PieResult<PieToken> {
        let child = token();
        record(Event::Derive(src, permission, mark, range, child));
        if crate::test_backend::fail(Op::Derive) {
            Err(make_fail(PieFail::Denied))
        } else {
            Ok(child)
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
}

pub mod pole {
    use crate::{FlatReference, PieToken, PoleFail, PoleResult, VirtAddr, make_fail};
    use crate::test_backend::{Event, Op, record, token};

    pub fn unseal(reference: FlatReference, shared: bool) -> PoleResult<PieToken> {
        let token = token();
        record(Event::Pole(reference.get(), shared, token));
        if crate::test_backend::fail(Op::Unseal) {
            Err(make_fail(PoleFail::Denied))
        } else {
            Ok(token)
        }
    }

    pub fn open(_: PieToken) -> PoleResult<(VirtAddr, usize)> {
        Ok((VirtAddr::new(0), 0))
    }
}

pub mod hole {
    pub(crate) use crate::hole_handle::Hole;
    use crate::{
        HoleCondition, HoleFail, HoleResult, HoleLimits, Mark, PieToken, TaskId, VirtAddr, Wait, make_fail,
        test_backend::{Event, Op, pop_message, record, token},
    };

    pub fn unseal(mark: Mark, limits: HoleLimits) -> HoleResult<PieToken> {
        assert_eq!(limits, HoleLimits::default());
        let token = token();
        record(Event::Unseal(token, mark));
        if crate::test_backend::fail(Op::Unseal) {
            Err(make_fail(HoleFail::Denied))
        } else {
            Ok(token)
        }
    }

    pub fn seal(token: PieToken) -> HoleResult<()> {
        record(Event::Seal(token));
        if crate::test_backend::fail(Op::Seal) {
            Err(make_fail(HoleFail::Denied))
        } else {
            Ok(())
        }
    }

    pub fn pull(
        token: PieToken,
        buf: VirtAddr,
        max: usize,
        oversize: crate::Oversize,
    ) -> HoleResult<crate::PullOutcome> {
        record(Event::Pull(token));
        let (bytes, from) = pop_message().ok_or_else(|| make_fail(HoleFail::Busy))?;
        if bytes.len() > max {
            return if oversize == crate::Oversize::Discard {
                Ok(crate::PullOutcome::Discarded {
                    len: bytes.len(),
                    sender: from,
                })
            } else {
                crate::test_backend::return_message(bytes, from);
                Err(make_fail(HoleFail::Denied))
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

    pub fn push(_: PieToken, _: VirtAddr, _: usize) -> HoleResult<()> {
        Ok(())
    }
    pub fn peek(_: PieToken) -> HoleResult<(usize, TaskId, usize)> {
        Err(make_fail(HoleFail::Busy))
    }
    pub fn wait(token: PieToken, condition: HoleCondition, within: Wait) -> HoleResult<bool> {
        record(Event::Wait(token, condition, within));
        if let Wait::AtMost(ms) = within {
            crate::test_backend::advance(ms as u64 * 1_000_000);
        }
        Ok(false)
    }
    pub fn ring(_: PieToken) -> HoleResult<()> {
        Ok(())
    }
    pub fn hush(_: PieToken) -> HoleResult<()> {
        Ok(())
    }
}

pub mod memory {
    use crate::test_backend::{self, Event, Op};
    use crate::{AcquireSource, FlatReference, MemoryFail, MemoryResult, PieToken, make_fail};
    pub fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference> {
        assert_eq!(retention, PieToken::NONE);
        let AcquireSource::Allocation(allocation) = source else {
            panic!("unexpected source")
        };
        test_backend::record(Event::Acquire(allocation.size));
        if test_backend::fail(Op::Acquire) {
            Err(make_fail(MemoryFail::OoM))
        } else {
            Ok(FlatReference::from_bytes(&7usize.to_le_bytes()).unwrap())
        }
    }
    pub fn release(reference: FlatReference) -> MemoryResult<()> {
        test_backend::record(Event::ReleaseReference(reference.get()));
        Ok(())
    }
}

pub mod test_backend {
    use abi::{HoleCondition, Mark, Permission, PieToken, TaskId, Wait};
    use std::cell::RefCell;
    use std::collections::VecDeque;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Op {
        Acquire,
        Unseal,
        Release,
        Seal,
        Accord,
        Derive,
        Revoke,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Event {
        Acquire(usize),
        ReleaseReference(usize),
        Pole(usize, bool, PieToken),
        Starve,
        Unseal(PieToken, Mark),
        Release(PieToken),
        Seal(PieToken),
        Accord(PieToken, TaskId, Permission, Mark, PieToken),
        Derive(PieToken, Permission, Mark, abi::GrantRange, PieToken),
        Revoke(TaskId, PieToken),
        Pull(PieToken),
        Wait(PieToken, HoleCondition, Wait),
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

    pub fn now() -> u64 {
        STATE.with(|state| state.borrow().now)
    }
    pub fn advance(ns: u64) {
        STATE.with(|state| state.borrow_mut().now += ns);
    }
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

#[path = "../../src/kernel.rs"]
pub mod kernel;
#[path = "../../src/cleanup.rs"]
pub mod cleanup;
#[path = "native.rs"]
pub mod native;
#[path = "../../src/capability.rs"]
pub mod capability;
#[path = "../../src/loan.rs"]
pub mod loan;
#[path = "../../src/hole.rs"]
#[allow(dead_code)]
pub(crate) mod hole_handle;
#[path = "../../src/raw.rs"]
pub mod raw;
#[path = "../../src/reply_error.rs"]
mod reply_error;
#[path = "../../src/reply.rs"]
pub mod reply;

#[cfg(test)]
mod tests;
