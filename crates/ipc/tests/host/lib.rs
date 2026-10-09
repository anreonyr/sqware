#![allow(dead_code)]

extern crate self as env;
extern crate self as resource;

pub use abi::{UnsealArgs, ReleaseMode, Oversize, PullOutcome,
    Access, MailCondition, MailFail, MailResult, Mark, Permission, PieFail, PieResult, PieToken,
    Policy, TaskId, VirtAddr, Wait, make_fail,
};

mod test_backend {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, vec::Vec};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Event {
        Open(PieToken, Mark),
        Grant(PieToken, TaskId, PieToken),
        Push(PieToken, Vec<u8>, Wait),
        Pull(PieToken, Wait),
        Seal(PieToken),
        Release(PieToken),
        Revoke(TaskId, PieToken),
        HandSend(PieToken, Vec<u8>, Wait),
    }

    pub struct State {
        pub next: usize,
        pub now: u64,
        pub peer: TaskId,
        pub reservation: Option<(TaskId, TaskId, Mark)>,
        pub events: Vec<Event>,
        pub opened: Vec<PieToken>,
        pub remotes: HashMap<PieToken, PieToken>,
        pub last_remote: Option<PieToken>,
        pub replies: HashMap<PieToken, (Vec<u8>, TaskId)>,
        pub automatic_reply: Option<(Vec<u8>, TaskId)>,
        pub fail_open: bool,
        pub fail_grant: bool,
        pub fail_send: bool,
        pub fail_pull: bool,
        pub fail_hand_send: bool,
        pub alive: bool,
        pub reply_error: Option<port::ReplyError>,
        pub push_advance_ns: u64,
    }

    impl Default for State {
        fn default() -> Self {
            Self { next: 100, now: 0, peer: TaskId::new(7), reservation: None, events: Vec::new(), opened: Vec::new(),
                remotes: HashMap::new(), last_remote: None, replies: HashMap::new(), automatic_reply: None,
                fail_open: false, fail_grant: false, fail_send: false, fail_pull: false,
                fail_hand_send: false, alive: true, reply_error: None, push_advance_ns: 0 }
        }
    }

    thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

    pub fn reset() { STATE.with(|state| *state.borrow_mut() = State::default()); }
    pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R { STATE.with(|state| f(&mut state.borrow_mut())) }
    pub fn event(event: Event) { with(|state| state.events.push(event)); }
    pub fn token() -> PieToken { with(|state| { let token = PieToken::mint(state.next); state.next += 1; token }) }
    pub fn now() -> u64 { with(|state| state.now) }
}

pub mod chrono {
    pub fn clock() -> u64 { crate::test_backend::now() }
}

pub mod pie {
    pub fn unseal(args: crate::UnsealArgs) -> PieResult<PieToken> {
        match args { crate::UnsealArgs::Hole { mark, .. } => hole_token(mark), _ => unreachable!() }
    }

    use crate::{Mark, PieFail, PieResult, PieToken, TaskId, make_fail, test_backend::{Event, event, token, with}};
    fn hole_token(_: Mark) -> PieResult<PieToken> { Ok(token()) }
    pub fn release(token: PieToken, _mode: crate::ReleaseMode) -> PieResult<()> { event(Event::Release(token)); Ok(()) }
    pub fn seal(token: PieToken) -> PieResult<()> { event(Event::Seal(token)); Ok(()) }
    pub fn revoke(peer: TaskId, remote: PieToken) -> PieResult<()> { event(Event::Revoke(peer, remote)); Ok(()) }
    pub fn accord(_: PieToken, _: TaskId, _: crate::Permission, _: Mark) -> PieResult<PieToken> { Ok(token()) }
    pub fn inspect(_: PieToken, _: crate::VirtAddr) -> PieResult<()> { Err(make_fail(PieFail::Denied)) }
    pub fn open(_: PieToken) -> PieResult<(crate::VirtAddr, usize)> { Ok((crate::VirtAddr::new(0), 0)) }
    pub fn set_now(now: u64) { with(|state| state.now = now); }
}

pub mod raw {
    use crate::{MailFail, MailResult, Mark, PieResult, PieToken, TaskId, Wait, make_fail, test_backend::with};
    pub struct Hole(PieToken);
    impl Hole {
        pub fn from_raw(token: PieToken) -> Self { Self(token) }
        pub fn pull(&self, buffer: &mut [u8], wait: Wait) -> MailResult<(usize, TaskId)> {
            let (bytes, from) = with(|state| {
                state.events.push(crate::test_backend::Event::Pull(self.0, wait));
                state.replies.remove(&self.0)
            }).ok_or_else(|| make_fail(MailFail::Busy))?;
            if bytes.len() > buffer.len() {
                with(|state| { state.replies.insert(self.0, (bytes, from)); });
                return Err(make_fail(MailFail::Denied));
            }
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok((bytes.len(), from))
        }
        pub fn pull_with(&self, buffer: &mut [u8], wait: Wait, oversize: crate::Oversize) -> MailResult<crate::PullOutcome> {
            match self.pull(buffer, wait) {
                Ok((len, sender)) => Ok(crate::PullOutcome::Received { len, sender }),
                Err(error) if error.source == MailFail::Denied && oversize == crate::Oversize::Discard => {
                    let (bytes, sender) = with(|state| state.replies.remove(&self.0)).unwrap();
                    Ok(crate::PullOutcome::Discarded { len: bytes.len(), sender })
                }
                Err(error) => Err(error),
            }
        }

    }
    pub fn release(token: PieToken) -> PieResult<()> { crate::pie::release(token, env::ReleaseMode::Revoke) }
    pub fn reserve(_: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
        Ok(with(|state| state.reservation.unwrap_or((state.peer, state.peer, Mark::of("back")))))
    }
    pub fn alive(_: PieToken) -> bool { with(|state| state.alive) }
}

pub mod port {
    use crate::{MailFail, MailResult, Mark, PieFail, PieResult, PieToken, TaskId, Wait, make_fail, test_backend::{Event, token, with}};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ReplyError { Mail(MailFail), WrongSource }

    pub struct Sender { entry: PieToken, peer: TaskId }
    impl Sender {
        pub fn import(entry: PieToken) -> PieResult<Self> {
            let (peer, fail) = with(|state| (state.peer, state.fail_open));
            if fail { Err(make_fail(PieFail::Denied)) } else { Ok(Self { entry, peer }) }
        }
        pub fn peer(&self) -> TaskId { self.peer }
        pub fn push(&self, bytes: &[u8], wait: Wait) -> MailResult<()> {
            let result = with(|state| {
                state.events.push(Event::Push(self.entry, bytes.to_vec(), wait));
                state.now = state.now.saturating_add(state.push_advance_ns);
                if state.fail_send { return Err(()); }
                if let Some((reply, from)) = state.automatic_reply.clone() {
                    if let Some(remote) = state.last_remote {
                        if let Some(local) = state.remotes.get(&remote).copied() {
                            state.replies.insert(local, (reply, from));
                        }
                    }
                }
                Ok(())
            });
            result.map_err(|_| make_fail(MailFail::Busy))
        }
    }

    pub struct Reply { local: PieToken, peer: TaskId, remote: Option<PieToken> }
    impl Reply {
        pub fn open(peer: TaskId, mark: Mark) -> PieResult<Self> {
            let fail = with(|state| state.fail_open);
            if fail { return Err(make_fail(PieFail::Denied)); }
            let local = token();
            with(|state| { state.opened.push(local); state.events.push(Event::Open(local, mark)); });
            Ok(Self { local, peer, remote: None })
        }
        pub fn grant(&mut self) -> PieResult<PieToken> {
            if self.remote.is_some() || with(|state| state.fail_grant) { return Err(make_fail(PieFail::Denied)); }
            let remote = token();
            with(|state| { state.remotes.insert(remote, self.local); state.last_remote = Some(remote); state.events.push(Event::Grant(self.local, self.peer, remote)); });
            self.remote = Some(remote);
            Ok(remote)
        }
        pub fn pull<'a>(&self, buffer: &'a mut [u8], wait: Wait) -> Result<&'a [u8], ReplyError> {
            let result = with(|state| {
                state.events.push(Event::Pull(self.local, wait));
                if state.fail_pull { return Err(ReplyError::Mail(MailFail::Busy)); }
                if let Some(error) = state.reply_error { return Err(error); }
                state.replies.remove(&self.local).ok_or(ReplyError::Mail(MailFail::Busy))
            })?;
            let (bytes, from) = result;
            if from != self.peer { return Err(ReplyError::WrongSource); }
            if bytes.len() > buffer.len() { return Err(ReplyError::Mail(MailFail::Denied)); }
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(&buffer[..bytes.len()])
        }
    }
    impl Drop for Reply {
        fn drop(&mut self) {
            if let Some(remote) = self.remote.take() { let _ = crate::pie::revoke(self.peer, remote); }
            with(|state| state.events.push(Event::Seal(self.local)));
            let _ = crate::pie::release(self.local, env::ReleaseMode::Revoke);
        }
    }
}

mod hand {
    use crate::{MailFail, PieToken, Wait, test_backend::{Event, event}};
    use wire::Message;
    pub enum SendFail { Mail(MailFail), TooLong, Unbound }
    pub struct Sender<M: Message> { token: PieToken, _message: core::marker::PhantomData<M> }
    impl<M: Message> Sender<M> {
        pub fn from_raw(token: PieToken) -> Self { Self { token, _message: core::marker::PhantomData } }
        pub fn send_within(&mut self, message: M, wait: Wait) -> Result<(), SendFail> {
            let mut buffer = M::EMPTY;
            let len = message.store(buffer.as_mut()).ok_or(SendFail::TooLong)?;
            let bytes = buffer.as_ref().get(..len).ok_or(SendFail::TooLong)?;
            event(Event::HandSend(self.token, bytes.to_vec(), wait));
            if crate::test_backend::with(|state| state.fail_hand_send) {
                return Err(SendFail::Mail(MailFail::Busy));
            }
            Ok(())
        }
    }
}

#[path = "../../src/time.rs"]
mod time;
#[path = "../../src/rpc.rs"]
mod rpc;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;
    use wire::Message;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Response(u8);
    impl Message for Response {
        type In = Self;
        type Buf = [u8; 1];
        const EMPTY: Self::Buf = [0];
        fn store(&self, out: &mut [u8]) -> Option<usize> { *out.first_mut()? = self.0; Some(1) }
        fn fetch(bytes: &[u8]) -> Option<Self> { Some(Self(*bytes.first()?)).filter(|_| bytes.len() == 1) }
    }

    struct TestContract;
    impl rpc::Contract for TestContract {
        type Request = Request;
        type Response = Response;
    }
    const BACK: Mark = Mark::of("back");
    fn route(request: &Request) -> PieToken { request.seed }

    fn client() -> rpc::request::Sender<TestContract> {
        rpc::request::Sender::from_raw(PieToken::mint(1), BACK).unwrap()
    }
    #[derive(Debug)]
    struct Request { seed: PieToken, fail_encode: bool }
    impl Message for Request {
        type In = Self;
        type Buf = [u8; 9];
        const EMPTY: Self::Buf = [0; 9];
        fn store(&self, out: &mut [u8]) -> Option<usize> {
            if self.fail_encode { return None; }
            *out.get_mut(0)? = 7;
            out.get_mut(1..9)?.copy_from_slice(&self.seed.to_bytes());
            Some(9)
        }
        fn fetch(bytes: &[u8]) -> Option<Self> {
            if bytes.len() != 9 || bytes[0] != 7 { return None; }
            Some(Self { seed: PieToken::from_bytes(bytes[1..9].try_into().ok()?)?, fail_encode: false })
        }
    }
    fn call(client: &rpc::request::Sender<TestContract>, deadline: time::Deadline) -> Result<Response, rpc::Fail> {
        client.call(deadline, |seed| Request { seed, fail_encode: false })
    }
    fn events() -> Vec<test_backend::Event> { test_backend::with(|state| state.events.clone()) }
    fn replies_for(token: PieToken, bytes: &[u8], from: TaskId) {
        test_backend::with(|state| { state.replies.insert(token, (bytes.to_vec(), from)); });
    }

    #[test]
    fn deadline_preserves_floor_rounding_and_saturating_large_budgets() {
        test_backend::reset();
        assert_eq!(time::deadline(Wait::AtMost(usize::MAX)), u64::MAX);
        assert_eq!(time::remain(u64::MAX), Wait::Forever);
        let deadline = time::Deadline::new(Wait::AtMost(usize::MAX));
        test_backend::with(|state| state.now = 999_999);
        assert_eq!(deadline.remaining(), Wait::AtMost(usize::MAX));
        test_backend::with(|state| state.now = 1_000_000);
        assert_eq!(deadline.remaining(), Wait::AtMost(usize::MAX - 1));
    }

    #[test]
    fn server_receive_keeps_kernel_source_and_rejects_malformed_requests() {
        test_backend::reset();
        let entry = PieToken::mint(1);
        let caller = TaskId::new(42);
        test_backend::with(|state| state.reservation = Some((caller, caller, BACK)));
        let request = Request { seed: PieToken::mint(55), fail_encode: false };
        let mut bytes = Request::EMPTY;
        request.store(&mut bytes).unwrap();
        replies_for(entry, &bytes, caller);
        let incoming = rpc::request::Receiver::<TestContract>::from_raw(entry, BACK, route).receive(&mut bytes, Wait::POLL).unwrap();
        assert_eq!(incoming.from, caller);
        assert_eq!(incoming.request.seed, request.seed);
        incoming.reply.send(Response(8)).unwrap();
        replies_for(entry, &[0; 9], caller);
        assert!(matches!(rpc::request::Receiver::<TestContract>::from_raw(entry, BACK, route).receive(&mut bytes, Wait::POLL),
            Err(rpc::request::Rejected { fail: rpc::Fail::Decode, incoming: None })));
    }

    #[test]
    fn split_send_captures_budget_and_drop_revokes_before_seal_release() {
        test_backend::reset();
        test_backend::with(|state| {
            state.push_advance_ns = 20_000_000;
            state.automatic_reply = Some((vec![5], TaskId::new(7)));
        });
        let client = client();
        let deadline = time::Deadline::new(Wait::AtMost(100));
        let receiver = client.send(deadline, |seed| Request { seed, fail_encode: false }).unwrap();
        let local = test_backend::with(|state| state.opened[0]);
        let remote = test_backend::with(|state| state.last_remote.unwrap());
        test_backend::with(|state| state.now += 30_000_000);
        assert_eq!(receiver.receive(), Ok(Response(5)));
        let events = events();
        assert!(events.iter().any(|event| matches!(event, test_backend::Event::Push(_, bytes, Wait::AtMost(100)) if bytes[0] == 7)));
        assert!(events.contains(&test_backend::Event::Pull(local, Wait::AtMost(50))));
        let tail = &events[events.len() - 3..];
        assert_eq!(tail, [test_backend::Event::Revoke(TaskId::new(7), remote), test_backend::Event::Seal(local), test_backend::Event::Release(local)]);
    }

    #[test]
    fn receive_can_drain_a_frame_larger_than_the_contract_buffer() {
        test_backend::reset();
        let entry = PieToken::mint(1);
        let caller = TaskId::new(7);
        let receiver = rpc::request::Receiver::<TestContract>::from_raw(entry, BACK, route);
        let mut buffer = [0; 16];
        replies_for(entry, &[0; 10], caller);
        assert!(matches!(receiver.receive(&mut buffer, Wait::POLL),
            Err(rpc::request::Rejected { fail: rpc::Fail::Decode, incoming: None })));
        assert!(!test_backend::with(|state| state.replies.contains_key(&entry)));
        let request = Request { seed: PieToken::mint(55), fail_encode: false };
        let n = request.store(&mut buffer).unwrap();
        replies_for(entry, &buffer[..n], caller);
        let incoming = receiver.receive(&mut buffer, Wait::POLL).unwrap();
        assert_eq!(incoming.request.seed, request.seed);
    }

    #[test]
    fn abandoning_split_reply_endpoint_cleans_up_without_receiving() {
        test_backend::reset();
        let client = client();
        let receiver = client.send(time::Deadline::new(Wait::AtMost(20)), |seed| Request { seed, fail_encode: false }).unwrap();
        let local = test_backend::with(|state| state.opened[0]);
        let remote = test_backend::with(|state| state.last_remote.unwrap());
        drop(receiver);
        let events = events();
        assert!(!events.iter().any(|event| matches!(event, test_backend::Event::Pull(..))));
        assert_eq!(&events[events.len() - 3..], [test_backend::Event::Revoke(TaskId::new(7), remote), test_backend::Event::Seal(local), test_backend::Event::Release(local)]);
    }

    #[test]
    fn one_deadline_covers_push_and_reply_and_encodes_the_granted_seed() {
        test_backend::reset();
        test_backend::with(|state| {
            state.now = 1_000_000_000;
            state.push_advance_ns = 30_000_000;
            state.automatic_reply = Some((vec![42], TaskId::new(7)));
        });
        let client = client();
        let deadline = time::Deadline::new(Wait::AtMost(100));
        assert_eq!(call(&client, deadline), Ok(Response(42)));
        let events = events();
        assert!(events.iter().any(|event| matches!(event, test_backend::Event::Push(_, bytes, Wait::AtMost(100)) if bytes[0] == 7)));
        assert!(events.iter().any(|event| matches!(event, test_backend::Event::Pull(_, Wait::AtMost(70)) )));
        let seed = events.iter().find_map(|event| match event {
            test_backend::Event::Push(_, bytes, _) => Some(&bytes[1..9]), _ => None,
        }).unwrap();
        let granted = events.iter().find_map(|event| match event {
            test_backend::Event::Grant(_, _, remote) => Some(remote.to_bytes()), _ => None,
        }).unwrap();
        assert_eq!(seed, &granted);
    }

    #[test]
    fn wrong_reply_source_is_rejected_and_the_exchange_is_closed() {
        test_backend::reset();
        test_backend::with(|state| state.reply_error = Some(port::ReplyError::WrongSource));
        let client = client();
        let deadline = time::Deadline::new(Wait::AtMost(100));
        assert_eq!(call(&client, deadline), Err(rpc::Fail::WrongSource));
        let events = events();
        let local = events.iter().find_map(|event| match event { test_backend::Event::Open(local, _) => Some(*local), _ => None }).unwrap();
        assert!(events.contains(&test_backend::Event::Seal(local)));
        assert!(events.contains(&test_backend::Event::Release(local)));
        assert!(events.iter().any(|event| matches!(event, test_backend::Event::Revoke(_, _))));
    }

    #[test]
    fn native_denied_is_not_misreported_as_wrong_source_and_bad_bytes_are_malformed() {
        test_backend::reset();
        test_backend::with(|state| state.reply_error = Some(port::ReplyError::Mail(MailFail::Denied)));
        let denied_client = client();
        let deadline = time::Deadline::new(Wait::AtMost(100));
        assert_eq!(call(&denied_client, deadline), Err(rpc::Fail::Receive(MailFail::Denied)));

        test_backend::reset();
        test_backend::with(|state| state.automatic_reply = Some((vec![], TaskId::new(7))));
        let malformed_client = client();
        assert_eq!(call(&malformed_client, deadline), Err(rpc::Fail::Decode));
    }

    #[test]
    fn one_budget_covers_multiple_calls_and_successful_old_replies_stay_isolated() {
        test_backend::reset();
        test_backend::with(|state| {
            state.push_advance_ns = 30_000_000;
            state.automatic_reply = Some((vec![42], TaskId::new(7)));
        });
        let client = client();
        let deadline = time::Deadline::new(Wait::AtMost(100));
        assert_eq!(call(&client, deadline), Ok(Response(42)));
        let old_local = test_backend::with(|state| state.opened[0]);
        replies_for(old_local, &[99], TaskId::new(7));
        assert_eq!(call(&client, deadline), Ok(Response(42)));
        let pushes: Vec<_> = events().iter().filter_map(|event| match event {
            test_backend::Event::Push(_, _, wait) => Some(*wait), _ => None,
        }).collect();
        assert_eq!(pushes, [Wait::AtMost(100), Wait::AtMost(70)]);
        let pulls: Vec<_> = events().iter().filter_map(|event| match event {
            test_backend::Event::Pull(_, wait) => Some(*wait), _ => None,
        }).collect();
        assert_eq!(pulls, [Wait::AtMost(70), Wait::AtMost(40)]);
        assert!(test_backend::with(|state| state.replies.contains_key(&old_local)));
    }

    #[test]
    fn a_late_reply_for_a_timed_out_call_cannot_complete_the_next_call() {
        test_backend::reset();
        let client = client();
        let deadline = time::Deadline::new(Wait::AtMost(0));
        assert_eq!(call(&client, deadline), Err(rpc::Fail::Receive(MailFail::Busy)));
        let old = test_backend::with(|state| state.opened[0]);
        replies_for(old, &[11], TaskId::new(7));
        test_backend::with(|state| state.automatic_reply = Some((vec![42], TaskId::new(7))));
        let next = time::Deadline::new(Wait::AtMost(100));
        assert_eq!(call(&client, next), Ok(Response(42)));
        let opened = test_backend::with(|state| state.opened.clone());
        assert_eq!(opened.len(), 2);
        assert_ne!(opened[0], opened[1]);
        assert!(test_backend::with(|state| state.replies.contains_key(&old)));
    }

    #[test]
    fn reply_tokens_are_source_checked_and_consumed_once() {
        test_backend::reset();
        let token = PieToken::mint(55);
        assert!(matches!(rpc::reply::Sender::<Response>::from_raw(token, TaskId::new(8), Mark::of("back")), Err(rpc::Fail::Untrusted)));
        assert!(!events().contains(&test_backend::Event::Release(token)));

        let reply = rpc::reply::Sender::<Response>::from_raw(token, TaskId::new(7), Mark::of("back")).unwrap();
        assert_eq!(reply.send(Response(9)), Ok(()));
        let events = events();
        assert_eq!(events.iter().filter(|event| matches!(event, test_backend::Event::HandSend(..))).count(), 1);
        assert_eq!(events.iter().filter(|event| matches!(event, test_backend::Event::Release(released) if *released == token)).count(), 1);
    }

    #[test]
    fn server_rejects_untrusted_back_and_retains_decoded_request_without_releasing_it() {
        test_backend::reset();
        let entry = PieToken::mint(2);
        let caller = TaskId::new(7);
        let remote_back = PieToken::mint(67);
        test_backend::with(|state| state.reservation = Some((TaskId::new(8), caller, BACK)));
        let request = Request { seed: remote_back, fail_encode: false };
        let mut bytes = Request::EMPTY;
        request.store(&mut bytes).unwrap();
        replies_for(entry, &bytes, caller);
        let receiver = rpc::request::Receiver::<TestContract>::from_raw(entry, BACK, route);
        let rejected = match receiver.receive(&mut bytes, Wait::POLL) {
            Ok(_) => panic!("untrusted reply route accepted"),
            Err(rejected) => rejected,
        };
        assert_eq!(rejected.fail, rpc::Fail::Untrusted);
        let (from, decoded) = rejected.incoming.unwrap();
        assert_eq!(from, caller);
        assert_eq!(decoded.seed, remote_back);
        assert!(!events().contains(&test_backend::Event::Release(remote_back)));
    }

    #[test]
    fn dropping_a_queued_server_reply_releases_its_imported_route() {
        test_backend::reset();
        let entry = PieToken::mint(1);
        let caller = TaskId::new(7);
        let back = PieToken::mint(66);
        test_backend::with(|state| state.reservation = Some((caller, caller, BACK)));
        let request = Request { seed: back, fail_encode: false };
        let mut bytes = Request::EMPTY;
        request.store(&mut bytes).unwrap();
        replies_for(entry, &bytes, caller);
        let receiver = rpc::request::Receiver::<TestContract>::from_raw(entry, BACK, route);
        let incoming = receiver.receive(&mut bytes, Wait::POLL).unwrap();
        assert!(!events().contains(&test_backend::Event::Release(back)));
        drop(incoming);
        let events = events();
        assert_eq!(events.iter().filter(|event| matches!(event, test_backend::Event::Release(token) if *token == back)).count(), 1);
    }

    #[test]
    fn typed_reply_encode_and_send_failures_release_the_imported_route_once() {
        struct BadResponse;
        impl Message for BadResponse {
            type In = Self;
            type Buf = [u8; 1];
            const EMPTY: Self::Buf = [0];
            fn store(&self, _: &mut [u8]) -> Option<usize> { None }
            fn fetch(_: &[u8]) -> Option<Self> { None }
        }

        test_backend::reset();
        let token = PieToken::mint(56);
        let reply = rpc::reply::Sender::<BadResponse>::from_raw(token, TaskId::new(7), Mark::of("back")).unwrap();
        assert!(matches!(reply.send(BadResponse), Err(rpc::Fail::Encode)));
        assert_eq!(events().iter().filter(|event| matches!(event, test_backend::Event::Release(value) if *value == token)).count(), 1);

        test_backend::reset();
        test_backend::with(|state| state.fail_hand_send = true);
        let token = PieToken::mint(57);
        let reply = rpc::reply::Sender::<Response>::from_raw(token, TaskId::new(7), Mark::of("back")).unwrap();
        assert_eq!(reply.send(Response(3)), Err(rpc::Fail::Send(MailFail::Busy)));
        assert_eq!(events().iter().filter(|event| matches!(event, test_backend::Event::Release(value) if *value == token)).count(), 1);
    }

    #[test]
    fn foreign_reply_transferor_owner_or_mark_is_not_released() {
        let caller = TaskId::new(7);
        let foreign = TaskId::new(8);
        let mark = Mark::of("back");
        for facts in [(foreign, caller, mark), (caller, foreign, mark),
            (caller, caller, Mark::of("other"))] {
            test_backend::reset();
            test_backend::with(|state| state.reservation = Some(facts));
            let token = PieToken::mint(55);
            assert!(matches!(rpc::reply::Sender::<Response>::from_raw(token, caller, mark), Err(rpc::Fail::Untrusted)));
            assert!(!events().contains(&test_backend::Event::Release(token)));
        }
    }

    #[test]
    fn encoding_grant_send_and_receive_errors_clean_up_without_retry() {
        for stage in ["encode", "grant", "send", "receive"] {
            test_backend::reset();
            test_backend::with(|state| match stage {
                "grant" => state.fail_grant = true,
                "send" => state.fail_send = true,
                "receive" => state.fail_pull = true,
                _ => {}
            });
            let client = client();
            let result = client.call(time::Deadline::new(Wait::AtMost(20)), |seed| Request {
                seed,
                fail_encode: stage == "encode",
            });
            assert!(result.is_err(), "{stage}");
            let events = events();
            assert_eq!(events.iter().filter(|event| matches!(event, test_backend::Event::Push(..))).count(), usize::from(stage == "receive" || stage == "send"), "{stage}");
            let local = events.iter().find_map(|event| match event { test_backend::Event::Open(local, _) => Some(*local), _ => None });
            if let Some(local) = local {
                assert!(events.contains(&test_backend::Event::Seal(local)), "{stage}");
                assert!(events.contains(&test_backend::Event::Release(local)), "{stage}");
            }
        }
    }

    #[test]
    fn queued_reply_sender_reports_dead_route_and_releases_on_drop() {
        test_backend::reset();
        let token = PieToken::mint(58);
        let reply = rpc::reply::Sender::<Response>::from_raw(
            token,
            TaskId::new(7),
            Mark::of("back"),
        )
        .unwrap();
        assert!(reply.is_alive());
        test_backend::with(|state| state.alive = false);
        assert!(!reply.is_alive());
        drop(reply);
        assert_eq!(
            events()
                .iter()
                .filter(|event| matches!(event, test_backend::Event::Release(value) if *value == token))
                .count(),
            1,
        );
    }
    #[test]
    fn oversized_request_is_discarded_and_next_valid_request_is_received() {
        test_backend::reset();
        let entry = PieToken::mint(999);
        test_backend::with(|state| { state.replies.insert(entry, (vec![0; 65], TaskId::new(7))); });
        let receiver = rpc::request::Receiver::<TestContract>::from_raw(entry, BACK, route);
        let mut buffer = [0; 64];
        let rejected = match receiver.receive(&mut buffer, Wait::POLL) {
            Err(error) => error, Ok(_) => panic!("oversized request accepted"),
        };
        assert_eq!(rejected.fail, rpc::Fail::Decode);
        test_backend::with(|state| assert!(!state.replies.contains_key(&entry)));
        let request = Request { seed: PieToken::mint(123), fail_encode: false };
        let size = request.store(&mut buffer).unwrap();
        test_backend::with(|state| { state.replies.insert(entry, (buffer[..size].to_vec(), TaskId::new(7))); });
        let incoming = receiver.receive(&mut buffer, Wait::POLL).unwrap();
        assert_eq!(incoming.request.seed, request.seed);
        assert_eq!(incoming.from, TaskId::new(7));
    }

}
