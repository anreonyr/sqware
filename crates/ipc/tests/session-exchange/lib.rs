#![allow(dead_code)]

extern crate self as env;
extern crate self as resource;
extern crate self as wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait { AtMost(usize), Forever }
impl Wait { pub const POLL: Self = Self::AtMost(0); }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieToken(pub usize);
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
        static QUEUE: RefCell<VecDeque<Frame>> = const { RefCell::new(VecDeque::new()) };
        static LAST_PULL: RefCell<Option<(usize, Wait)>> = const { RefCell::new(None) };
        static PUSH_FAIL: RefCell<bool> = const { RefCell::new(false) };
        static PUSH_ADVANCE: RefCell<u64> = const { RefCell::new(0) };
        static PULLS: RefCell<usize> = const { RefCell::new(0) };
        static LAST_PUSH: RefCell<Option<PieToken>> = const { RefCell::new(None) };
    }
    pub struct Hole(PieToken);
    impl Hole {
        pub fn from_raw(token: PieToken) -> Self { Self(token) }
        pub fn pull(&self, buf: &mut [u8], wait: Wait) -> Result<(usize, TaskId), PullError> {
            let _ = self.0;
            PULLS.with(|n| *n.borrow_mut() += 1);
            LAST_PULL.with(|p| *p.borrow_mut() = Some((buf.len(), wait)));
            QUEUE.with(|q| {
                let f = q.borrow_mut().pop_front().ok_or(PullError { source: MailFail::Busy })?;
                if f.0.len() > buf.len() { return Err(PullError { source: MailFail::Busy }); }
                buf[..f.0.len()].copy_from_slice(&f.0);
                Ok((f.0.len(), f.1))
            })
        }
        pub fn push(&self, bytes: &[u8], _wait: Wait) -> Result<(), PushError> {
            LAST_PUSH.with(|p| *p.borrow_mut() = Some(self.0));
            if PUSH_FAIL.with(|v| v.replace(false)) { return Err(PushError { source: MailFail::Dead }); }
            let _ = bytes;
            super::chrono::advance(PUSH_ADVANCE.with(|v| v.replace(0)));
            Ok(())
        }
        pub fn depth(&self) -> Result<usize, PullError> { Ok(0) }
    }
    #[derive(Debug)] pub struct PullError { pub source: MailFail }
    #[derive(Debug)] pub struct PushError { pub source: MailFail }
    pub fn enqueue(bytes: Vec<u8>, from: TaskId) { QUEUE.with(|q| q.borrow_mut().push_back(Frame(bytes, from))); }
    pub fn last_pull() -> Option<(usize, Wait)> { LAST_PULL.with(|p| *p.borrow()) }
    pub fn last_push() -> Option<PieToken> { LAST_PUSH.with(|p| *p.borrow()) }
    pub fn pull_count() -> usize { PULLS.with(|p| *p.borrow()) }
    pub fn fail_next_push() { PUSH_FAIL.with(|v| *v.borrow_mut() = true); }
    pub fn advance_on_push(ns: u64) { PUSH_ADVANCE.with(|v| *v.borrow_mut() = ns); }
    pub fn reset() {
        QUEUE.with(|q| q.borrow_mut().clear()); LAST_PULL.with(|p| *p.borrow_mut() = None);
        PUSH_FAIL.with(|v| *v.borrow_mut() = false); PUSH_ADVANCE.with(|v| *v.borrow_mut() = 0);
        PULLS.with(|v| *v.borrow_mut() = 0); LAST_PUSH.with(|p| *p.borrow_mut() = None);
        super::chrono::reset();
    }
}

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
    pub struct Endpoint { pub sender: Option<PieToken>, pub receiver: PieToken }
    impl Endpoint {
        pub fn sender<M: Message>(&self) -> Option<Sender<M>> { self.sender.map(Sender::bound) }
        pub fn receiver<M: Message>(&self) -> Receiver<M> { Receiver::from_raw(self.receiver) }
    }
    pub struct Session { pub link: Endpoint, pub talk: PieToken, pub host: TaskId }
    pub mod exchange {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/session/exchange.rs"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::{exchange::{self, CallFail, Contract}, Endpoint, Session};
    struct ContractNum;
    impl Contract for ContractNum { type Request = Number; type Response = Number; }
    fn session(sender: Option<PieToken>) -> Session {
        Session { link: Endpoint { sender, receiver: PieToken(2) }, talk: PieToken(3), host: TaskId(42) }
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
        raw::reset(); raw::advance_on_push(3_000_000); raw::enqueue(vec![11], TaskId(42));
        assert_eq!(exchange::call::<ContractNum>(&session(Some(PieToken(1))), Number(4), Wait::AtMost(10)).unwrap(), 11);
        assert_eq!(raw::last_pull(), Some((1, Wait::AtMost(7))));
    }
    #[test]
    fn wrong_source_and_decode_remain_distinct_in_call() {
        raw::reset(); raw::enqueue(vec![0xff], TaskId(99));
        assert!(matches!(exchange::call::<ContractNum>(&session(Some(PieToken(1))), Number(4), Wait::POLL), Err(CallFail::Receive(receiver::SourceFail::WrongSource))));
        raw::reset(); raw::enqueue(vec![0xff], TaskId(42));
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
        let talk = s.talk;
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
}
