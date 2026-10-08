#![allow(dead_code)]

extern crate alloc;
extern crate self as env;
extern crate self as resource;
extern crate self as wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait { AtMost(usize), Forever }
impl Wait { pub const POLL: Self = Self::AtMost(0); }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(pub usize);
impl TaskId { pub const fn new(id: usize) -> Self { Self(id) } }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PieToken(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark(pub usize);
impl Mark { pub const NONE: Self = Self(0); }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailFail { Busy, Dead }

pub mod chrono {
    use std::cell::Cell;
    thread_local! { static NOW: Cell<u64> = const { Cell::new(0) }; }
    pub fn clock() -> u64 { NOW.with(Cell::get) }
    pub fn advance(ns: u64) { NOW.with(|n| n.set(n.get().saturating_add(ns))); }
    pub fn reset() { NOW.with(|n| n.set(0)); }
}

pub trait Message {
    type In;
    type Buf: AsRef<[u8]> + AsMut<[u8]>;
    const EMPTY: Self::Buf;
    fn store(&self, out: &mut [u8]) -> Option<usize>;
    fn fetch(bytes: &[u8]) -> Option<Self::In>;
}
pub trait Contract {
    type Request: Message;
    type Response: Message;
}
pub struct Number(pub u8);
impl Message for Number {
    type In = u8;
    type Buf = [u8; 1];
    const EMPTY: Self::Buf = [0];
    fn store(&self, out: &mut [u8]) -> Option<usize> { *out.get_mut(0)? = self.0; Some(1) }
    fn fetch(bytes: &[u8]) -> Option<Self::In> { (bytes.len() == 1 && bytes[0] != 0xff).then(|| bytes[0]) }
}

pub mod raw {
    use super::{MailFail, PieToken, TaskId, Wait};
    use std::{cell::RefCell, collections::VecDeque};
    #[derive(Clone)] struct Frame(Vec<u8>, TaskId);
    thread_local! {
        static QUEUE: RefCell<VecDeque<(PieToken, Frame)>> = const { RefCell::new(VecDeque::new()) };
        static LAST_PULL: RefCell<Option<(usize, Wait)>> = const { RefCell::new(None) };
        static PUSH_FAIL: RefCell<bool> = const { RefCell::new(false) };
        static PUSH_ADVANCE: RefCell<u64> = const { RefCell::new(0) };
        static PULLS: RefCell<usize> = const { RefCell::new(0) };
        static LAST_PUSH: RefCell<Option<PieToken>> = const { RefCell::new(None) };
        static DEAD: RefCell<std::collections::HashSet<PieToken>> = RefCell::new(std::collections::HashSet::new());
        static SEAL_FAIL: RefCell<bool> = const { RefCell::new(false) };
        static SEALS: RefCell<Vec<PieToken>> = const { RefCell::new(Vec::new()) };
        static NEXT_REPLY: RefCell<Option<(Vec<u8>, TaskId)>> = const { RefCell::new(None) };
        static PUSHES: RefCell<usize> = const { RefCell::new(0) };
        static RESERVE_OWNER: RefCell<usize> = const { RefCell::new(1) };
        static PUSH_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
        static PULL_PANIC: RefCell<bool> = const { RefCell::new(false) };
        static DEPTH_FAIL: RefCell<bool> = const { RefCell::new(false) };
    }
    pub struct Hole(PieToken);
    impl Hole {
        pub fn from_raw(token: PieToken) -> Self { Self(token) }
        pub fn pull(&self, buf: &mut [u8], wait: Wait) -> Result<(usize, TaskId), PullError> {
            PULLS.with(|n| *n.borrow_mut() += 1);
            if PULL_PANIC.with(|v| v.replace(false)) { panic!("mock receive interruption"); }
            LAST_PULL.with(|p| *p.borrow_mut() = Some((buf.len(), wait)));
            QUEUE.with(|q| {
                let f = {
                    let mut q = q.borrow_mut();
                    let pos = q.iter().position(|(token, _)| *token == self.0).ok_or(PullError { source: MailFail::Busy })?;
                    q.remove(pos).unwrap().1
                };
                if f.0.len() > buf.len() { return Err(PullError { source: MailFail::Busy }); }
                buf[..f.0.len()].copy_from_slice(&f.0);
                Ok((f.0.len(), f.1))
            })
        }
        pub fn push(&self, bytes: &[u8], _wait: Wait) -> Result<(), PushError> {
            LAST_PUSH.with(|p| *p.borrow_mut() = Some(self.0));
            if PUSH_FAIL.with(|v| v.replace(false)) { return Err(PushError { source: MailFail::Dead }); }
            PUSHES.with(|n| *n.borrow_mut() += 1);
            let _ = bytes;
            super::chrono::advance(PUSH_ADVANCE.with(|v| v.replace(0)));
            if let Some((bytes, from)) = NEXT_REPLY.with(|v| v.borrow_mut().take()) {
                QUEUE.with(|q| q.borrow_mut().push_back((PieToken(2), Frame(bytes, from))));
            }
            if let Some(hook) = PUSH_HOOK.with(|v| v.borrow_mut().take()) { hook(); }
            Ok(())
        }
        pub fn depth(&self) -> Result<usize, PullError> {
            if DEPTH_FAIL.with(|v| v.replace(false)) { return Err(PullError { source: MailFail::Dead }); }
            Ok(QUEUE.with(|q| q.borrow().iter().filter(|(token, _)| *token == self.0).count()))
        }
    }
    #[derive(Debug)] pub struct PullError { pub source: MailFail }
    #[derive(Debug)] pub struct PushError { pub source: MailFail }
    pub fn enqueue(bytes: Vec<u8>, from: TaskId) { enqueue_to(PieToken(1), bytes, from); }
    pub fn enqueue_to(token: PieToken, bytes: Vec<u8>, from: TaskId) { QUEUE.with(|q| q.borrow_mut().push_back((token, Frame(bytes, from)))); }
    pub fn reply_on_next_push(bytes: Vec<u8>, from: TaskId) { NEXT_REPLY.with(|v| *v.borrow_mut() = Some((bytes, from))); }
    pub fn last_pull() -> Option<(usize, Wait)> { LAST_PULL.with(|p| *p.borrow()) }
    pub fn last_push() -> Option<PieToken> { LAST_PUSH.with(|p| *p.borrow()) }
    pub fn pull_count() -> usize { PULLS.with(|p| *p.borrow()) }
    pub fn fail_next_push() { PUSH_FAIL.with(|v| *v.borrow_mut() = true); }
    pub fn advance_on_push(ns: u64) { PUSH_ADVANCE.with(|v| *v.borrow_mut() = ns); }
    pub fn reserve(_: PieToken) -> Result<(TaskId, TaskId, crate::Mark), ()> {
        Ok((TaskId(8), TaskId(RESERVE_OWNER.with(|v| *v.borrow())), crate::Mark(1)))
    }
    pub fn reset() {
        QUEUE.with(|q| q.borrow_mut().clear()); NEXT_REPLY.with(|v| *v.borrow_mut() = None); LAST_PULL.with(|p| *p.borrow_mut() = None);
        PUSH_FAIL.with(|v| *v.borrow_mut() = false); PUSH_ADVANCE.with(|v| *v.borrow_mut() = 0);
        PULLS.with(|v| *v.borrow_mut() = 0); LAST_PUSH.with(|p| *p.borrow_mut() = None);
        PUSHES.with(|v| *v.borrow_mut() = 0); RESERVE_OWNER.with(|v| *v.borrow_mut() = 1); PUSH_HOOK.with(|v| *v.borrow_mut() = None);
        PULL_PANIC.with(|v| *v.borrow_mut() = false);
        DEPTH_FAIL.with(|v| *v.borrow_mut() = false);
        DEAD.with(|v| v.borrow_mut().clear()); SEAL_FAIL.with(|v| *v.borrow_mut() = false); SEALS.with(|v| v.borrow_mut().clear());
        super::chrono::reset();
    }
    pub fn alive(token: PieToken) -> bool { DEAD.with(|v| !v.borrow().contains(&token)) }
    pub fn seal(token: PieToken) -> Result<(), ()> {
        SEALS.with(|v| v.borrow_mut().push(token));
        if SEAL_FAIL.with(|v| v.replace(false)) { return Err(()); }
        DEAD.with(|v| { v.borrow_mut().insert(token); });
        Ok(())
    }
    pub fn fail_next_seal() { SEAL_FAIL.with(|v| *v.borrow_mut() = true); }
    pub fn seals() -> Vec<PieToken> { SEALS.with(|v| v.borrow().clone()) }
    pub fn set_reserve_owner(owner: usize) { RESERVE_OWNER.with(|v| *v.borrow_mut() = owner); }
    pub fn pushes() -> usize { PUSHES.with(|v| *v.borrow()) }
    pub fn queue_depth(token: PieToken) -> usize { QUEUE.with(|q| q.borrow().iter().filter(|(t, _)| *t == token).count()) }
    pub fn on_next_push(hook: impl FnOnce() + 'static) { PUSH_HOOK.with(|v| *v.borrow_mut() = Some(Box::new(hook))); }
    pub fn panic_on_next_pull() { PULL_PANIC.with(|v| *v.borrow_mut() = true); }
    pub fn fail_next_depth() { DEPTH_FAIL.with(|v| *v.borrow_mut() = true); }
}
pub mod unit { use crate::TaskId; pub fn self_id() -> TaskId { TaskId(1) } }
pub mod pie { pub use crate::raw::{alive, seal}; }

#[path = "../../src/hand/receiver.rs"]
pub mod receiver;

pub mod hand {
    use crate::{MailFail, Message, PieToken, Wait};
    pub use crate::receiver::{Receiver, RecvFail, SourceFail};
    pub struct Sender<M: Message>(Option<PieToken>, core::marker::PhantomData<M>);
    impl<M: Message> Sender<M> {
        pub fn bound(token: PieToken) -> Self { Self(Some(token), core::marker::PhantomData) }
        pub fn from_raw(token: PieToken) -> Self { Self(Some(token), core::marker::PhantomData) }
        pub fn send_within(&mut self, msg: M, wait: Wait) -> Result<(), SendFail> {
            let token = self.0.ok_or(SendFail::Unbound)?;
            let mut buf = M::EMPTY;
            let n = msg.store(buf.as_mut()).ok_or(SendFail::TooLong)?;
            crate::raw::Hole::from_raw(token).push(&buf.as_ref()[..n], wait).map_err(|e| SendFail::Mail(e.source))
        }
    }
    #[derive(Debug)] pub enum SendFail { Unbound, TooLong, Mail(MailFail) }
}

#[path = "../../src/time.rs"]
pub mod time;

pub mod session {
    use crate::{hand::{Receiver, Sender}, Message, PieToken, TaskId};
    pub use crate::Contract;
    use alloc::sync::Arc;
    pub mod state {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/session/state.rs"));
    }
    #[derive(Clone, Copy)]
    pub struct Endpoint { pub sender: Option<PieToken>, pub receiver: PieToken }
    impl Endpoint {
        pub fn rx(&self) -> PieToken { self.receiver }
        pub fn sender<M: Message>(&self) -> Option<Sender<M>> { self.sender.map(Sender::bound) }
        pub fn receiver<M: Message>(&self) -> Receiver<M> { Receiver::from_raw(self.receiver) }
    }
    pub struct Session { link: Endpoint, talk: PieToken, host: TaskId, state: Arc<state::State> }
    impl Session {
        pub fn from_raw(link: Endpoint, talk: PieToken, host: TaskId) -> Result<Self, ()> {
            if !state::valid_reply(link.rx(), host) { return Err(()); }
            Ok(Self { link, talk, host, state: state::new_state() })
        }
        pub fn link(&self) -> Endpoint { self.link }
        pub fn talk(&self) -> PieToken { self.talk }
        pub fn host(&self) -> TaskId { self.host }
        pub fn clone_alias(&self) -> Self { Self { link: self.link, talk: self.talk, host: self.host, state: Arc::clone(&self.state) } }
    }
    pub mod exchange {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/session/exchange.rs"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::{exchange::{self, CallFail}, Contract, Endpoint, Session};
    struct ContractNum;
    impl Contract for ContractNum { type Request = Number; type Response = Number; }
    fn session(sender: Option<PieToken>) -> Session {
        Session::from_raw(Endpoint { sender, receiver: PieToken(2) }, PieToken(3), TaskId(42)).ok().unwrap()
    }

    #[test]
    fn wrong_source_is_checked_before_decode() {
        raw::reset(); raw::enqueue(vec![0xff, 0xfe], TaskId(9));
        let rx = receiver::Receiver::<Number>::from_raw(PieToken(1));
        assert!(matches!(rx.recv_from(TaskId(42), &mut [0; 64], Wait::POLL), Err(receiver::SourceFail::WrongSource)));
        assert_eq!(raw::pull_count(), 1);
    }
    #[test]
    fn malformed_expected_frame_reports_unread() {
        raw::reset(); raw::enqueue(vec![0xff], TaskId(42));
        let rx = receiver::Receiver::<Number>::from_raw(PieToken(1));
        assert!(matches!(rx.recv_from(TaskId(42), &mut [0; 64], Wait::POLL), Err(receiver::SourceFail::Receive(receiver::RecvFail::Unread(1)))));
    }
    #[test]
    fn exchange_shares_deadline_and_accepts_valid_reply() {
        raw::reset(); raw::advance_on_push(3_000_000); raw::reply_on_next_push(vec![11], TaskId(42));
        assert_eq!(exchange::call::<ContractNum>(&session(Some(PieToken(1))), Number(4), Wait::AtMost(10)).unwrap(), 11);
        assert_eq!(raw::last_pull(), Some((1, Wait::AtMost(7))));
    }
    #[test]
    fn wrong_source_and_decode_remain_distinct_in_call() {
        raw::reset(); raw::reply_on_next_push(vec![0xff], TaskId(99));
        assert!(matches!(exchange::call::<ContractNum>(&session(Some(PieToken(1))), Number(4), Wait::POLL), Err(CallFail::Receive(receiver::SourceFail::WrongSource))));
        raw::reset(); raw::reply_on_next_push(vec![0xff], TaskId(42));
        let result = exchange::call::<ContractNum>(&session(Some(PieToken(1))), Number(4), Wait::POLL);
        assert!(matches!(result, Err(CallFail::Receive(receiver::SourceFail::Receive(receiver::RecvFail::Unread(1))))), "{result:?}");
    }
    #[test]
    fn send_failure_never_receives_or_retries() {
        raw::reset(); raw::fail_next_push();
        assert!(matches!(exchange::call::<ContractNum>(&session(Some(PieToken(1))), Number(4), Wait::POLL), Err(CallFail::Send(hand::SendFail::Mail(MailFail::Dead)))));
        assert_eq!(raw::pull_count(), 0);
    }
    #[test]
    fn request_uses_talk_even_without_endpoint_transmit_half() {
        raw::reset();
        let s = session(None);
        let talk = s.talk();
        assert!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL).is_err());
        assert_eq!(raw::last_push(), Some(talk));
        assert_eq!(raw::pull_count(), 1);
    }
    #[test]
    fn receiver_uses_caller_buffer_capacity() {
        raw::reset(); raw::enqueue(vec![5], TaskId(42));
        let rx = receiver::Receiver::<Number>::from_raw(PieToken(1));
        assert_eq!(rx.recv_from(TaskId(42), &mut [0; 64], Wait::POLL).unwrap(), 5);
        assert_eq!(raw::last_pull().unwrap().0, 64);
    }

    #[test]
    fn aliases_share_closed_state_and_late_reply_is_not_consumed() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        let alias = s.clone_alias();
        assert!(matches!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL), Err(CallFail::Receive(_))));
        assert_eq!(raw::seals(), vec![PieToken(2)]);
        raw::enqueue_to(PieToken(2), vec![11], TaskId(42));
        assert!(matches!(exchange::call::<ContractNum>(&alias, Number(4), Wait::POLL), Err(CallFail::Closed)));
        assert_eq!(raw::pushes(), 1);
        assert_eq!(raw::queue_depth(PieToken(2)), 1);
    }

    #[test]
    fn queued_duplicate_reply_closes_before_a_second_request() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        raw::reply_on_next_push(vec![11], TaskId(42));
        assert_eq!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL).unwrap(), 11);
        raw::enqueue_to(PieToken(2), vec![12], TaskId(42));
        assert!(matches!(exchange::call::<ContractNum>(&s, Number(5), Wait::POLL), Err(CallFail::Closed)));
        assert_eq!(raw::pushes(), 1);
        assert_eq!(raw::seals(), vec![PieToken(2)]);
    }

    #[test]
    fn send_failure_restores_ready_for_a_later_exchange() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        raw::fail_next_push();
        assert!(matches!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL), Err(CallFail::Send(_))));
        raw::reply_on_next_push(vec![13], TaskId(42));
        assert_eq!(exchange::call::<ContractNum>(&s, Number(5), Wait::POLL).unwrap(), 13);
        assert_eq!(raw::pushes(), 1);
        assert!(raw::seals().is_empty());
    }

    #[test]
    fn concurrent_alias_call_returns_busy_without_sending() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        let alias = s.clone_alias();
        let nested = std::rc::Rc::new(std::cell::RefCell::new(None));
        let result = nested.clone();
        raw::on_next_push(move || {
            *result.borrow_mut() = Some(exchange::call::<ContractNum>(&alias, Number(9), Wait::POLL));
        });
        raw::reply_on_next_push(vec![14], TaskId(42));
        assert_eq!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL).unwrap(), 14);
        assert!(matches!(nested.borrow().as_ref().unwrap(), Err(CallFail::Busy)));
        assert_eq!(raw::pushes(), 1);
    }

    #[test]
    fn seal_failure_keeps_alias_closed_and_returns_receive_error() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        let alias = s.clone_alias();
        raw::fail_next_seal();
        assert!(matches!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL), Err(CallFail::Receive(_))));
        assert!(raw::alive(PieToken(2)));
        assert!(matches!(exchange::call::<ContractNum>(&alias, Number(5), Wait::POLL), Err(CallFail::Closed)));
        assert_eq!(raw::pushes(), 1);
    }

    #[test]
    fn preclosed_or_unowned_raw_reply_is_rejected() {
        raw::reset();
        assert!(session::Session::from_raw(Endpoint { sender: None, receiver: PieToken(2) }, PieToken(3), TaskId(0)).is_err());
        raw::set_reserve_owner(99);
        assert!(session::Session::from_raw(Endpoint { sender: None, receiver: PieToken(2) }, PieToken(3), TaskId(42)).is_err());
        raw::set_reserve_owner(1);
        raw::seal(PieToken(2)).unwrap();
        assert!(session::Session::from_raw(Endpoint { sender: None, receiver: PieToken(2) }, PieToken(3), TaskId(42)).is_err());
    }

    #[test]
    fn unwind_after_send_closes_and_seals_reply_endpoint() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        let alias = s.clone_alias();
        raw::reply_on_next_push(vec![15], TaskId(42));
        raw::panic_on_next_pull();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = exchange::call::<ContractNum>(&s, Number(4), Wait::POLL);
        }));
        assert!(result.is_err());
        assert!(!raw::alive(PieToken(2)));
        assert_eq!(raw::seals(), vec![PieToken(2)]);
        assert!(matches!(exchange::call::<ContractNum>(&alias, Number(5), Wait::POLL), Err(CallFail::Closed)));
        assert_eq!(raw::pushes(), 1);
    }

    #[test]
    fn closed_talk_or_failed_depth_check_closes_without_sending() {
        raw::reset();
        let s = session(Some(PieToken(1)));
        raw::seal(PieToken(3)).unwrap();
        assert!(matches!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL), Err(CallFail::Closed)));
        assert_eq!(raw::pushes(), 0);
        assert!(!raw::alive(PieToken(2)));

        raw::reset();
        let s = session(Some(PieToken(1)));
        raw::fail_next_depth();
        assert!(matches!(exchange::call::<ContractNum>(&s, Number(4), Wait::POLL), Err(CallFail::Closed)));
        assert_eq!(raw::pushes(), 0);
        assert_eq!(raw::seals(), vec![PieToken(2)]);
    }
}
