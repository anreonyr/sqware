#![allow(dead_code)]
extern crate alloc;
extern crate env as abi_env;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

use core::marker::PhantomData;
use std::{cell::RefCell, vec::Vec};
pub use abi_env::{Mark, PieToken, TaskId, Wait};
use wire::message::Message;

thread_local! {
    static CAPTURE: RefCell<Option<(Vec<u8>, Wait, PieToken)>> = const { RefCell::new(None) };
    static PUBLICATION: RefCell<PublicationState> = const { RefCell::new(PublicationState::new()) };
}

struct PublicationState {
    fail_send: bool,
    fail_receive: bool,
    seed: Option<PieToken>,
    revoked: Vec<(TaskId, PieToken)>,
}
impl PublicationState {
    const fn new() -> Self { Self { fail_send: false, fail_receive: false, seed: None, revoked: Vec::new() } }
}

pub mod pie {
    use crate::{PieToken, TaskId, PUBLICATION};
    pub fn revoke(peer: TaskId, token: PieToken) -> Result<(), ()> {
        PUBLICATION.with(|s| s.borrow_mut().revoked.push((peer, token)));
        Ok(())
    }
}
pub mod unit {
    use crate::TaskId;
    pub fn self_id() -> TaskId { TaskId::new(1) }
    pub fn sire() -> TaskId { TaskId::new(7) }
}

pub mod raw {
    use crate::{Mark, PieToken, TaskId};
    pub fn reserve(_: PieToken) -> Result<(TaskId, TaskId, Mark), ()> {
        Ok((TaskId::new(7), TaskId::new(7), system_api::control::publication::ENTRY))
    }
    pub fn inspect(_: PieToken) -> Result<(TaskId, TaskId, Mark), ()> {
        Ok((TaskId::new(1), TaskId::new(1), Mark::NONE))
    }
}

pub mod control {
    pub use system_api::control::{frame, marks, publication, Fail, Grant, Req, Request, Said, State, Wire};
    pub mod account { pub use system_api::control::account::*; }
    pub mod client {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../crates/system-client/src/control/client.rs"));
    }
}

pub mod identity {
    pub use system_api::identity::{CoalitionId, PrincipalId};
}

pub mod loader {
    use crate::TaskId;
    pub mod frame { pub use system_api::loader::Said; }
    pub struct Built { pub task: TaskId, pub team: abi_env::TeamId }
}

pub mod operator {
    pub use system_api::operator::{EntryId, Fail, Permit, path::{Path, PathBuf}};
    pub mod client {
        pub struct Face;
        pub struct Root;
        pub struct Tile;
        impl Face { pub fn root(&self) -> Root { Root } }
        impl Root {
            pub fn tile(&self, _: &system_api::operator::path::PathBuf, _: env::Wait)
                -> Result<Tile, system_api::operator::Fail> { Err(system_api::operator::Fail::Unknown) }
        }
        impl Tile {
            pub fn token(&self, _: env::Wait) -> Result<env::PieToken, system_api::operator::Fail> {
                Err(system_api::operator::Fail::Unknown)
            }
        }
    }
    pub use client::Face;
}

pub mod debug { pub fn put(_: &str) {} }
pub mod port {
    use crate::{PieToken, TaskId, PUBLICATION};
    #[derive(Clone, Copy)] pub struct Access;
    impl Access { pub const FETCH: Self = Self; pub const STORE: Self = Self; }
    impl core::ops::BitOr for Access { type Output = Self; fn bitor(self, _: Self) -> Self { self } }
    #[derive(Clone, Copy)] pub struct Policy;
    impl Policy { pub const VEST: Self = Self; }
    pub struct To(PieToken);
    impl To { pub fn seed(self) -> PieToken { self.0 } }
    pub fn ship(_: PieToken, _: TaskId, _: Access, _: Policy) -> Result<To, ()> {
        let seed = PieToken::from_bytes(&55u64.to_le_bytes()).unwrap();
        PUBLICATION.with(|s| s.borrow_mut().seed = Some(seed));
        Ok(To(seed))
    }
}

pub mod rpc {
    use super::*;

    #[derive(Debug)]
    pub enum Fail { Open(()), Grant(()), Encode, Decode, Send(()), Receive(()), Untrusted, WrongSource }

    pub mod request {
        use super::*;

        pub struct Sender<C: wire::Contract> { entry: PieToken, _contract: PhantomData<C> }
        impl<C: wire::Contract> Sender<C> {
            pub fn from_raw(entry: PieToken, _: Mark) -> Result<Self, Fail> {
                Ok(Self { entry, _contract: PhantomData })
            }
            pub fn peer(&self) -> TaskId { TaskId::new(7) }
            pub fn call(
                &self,
                deadline: crate::time::Deadline,
                build: impl FnOnce(PieToken) -> C::Request,
            ) -> Result<<C::Response as Message>::In, Fail> {
                let back = PieToken::from_bytes(&88u64.to_le_bytes()).unwrap();
                let request = build(back);
                let mut buffer = C::Request::EMPTY;
                let len = request.store(buffer.as_mut()).ok_or(Fail::Encode)?;
                let bytes = buffer.as_ref().get(..len).ok_or(Fail::Encode)?.to_vec();
                CAPTURE.with(|capture| *capture.borrow_mut() = Some((bytes, deadline.wait, back)));
                Err(Fail::Send(()))
            }

            pub fn send(
                &self,
                deadline: crate::time::Deadline,
                build: impl FnOnce(PieToken) -> C::Request,
            ) -> Result<reply::Receiver<C::Response>, Fail> {
                let back = PieToken::from_bytes(&99u64.to_le_bytes()).unwrap();
                let request = build(back);
                let mut buffer = C::Request::EMPTY;
                let len = request.store(buffer.as_mut()).ok_or(Fail::Encode)?;
                let bytes = buffer.as_ref().get(..len).ok_or(Fail::Encode)?.to_vec();
                CAPTURE.with(|capture| *capture.borrow_mut() = Some((bytes, deadline.wait, back)));
                if PUBLICATION.with(|s| s.borrow().fail_send) { return Err(Fail::Send(())); }
                Ok(super::reply::Receiver(PhantomData))
            }
        }
    }

    pub mod reply {
        use super::*;
        pub struct Receiver<R: Message>(pub PhantomData<R>);
        impl<R: Message> Receiver<R> {
            pub fn receive(self) -> Result<R::In, Fail> {
                let fail = PUBLICATION.with(|s| s.borrow().fail_receive);
                if fail { Err(Fail::Receive(())) } else { Err(Fail::Decode) }
            }
        }
    }
}

pub mod time {
    use env::Wait;
    #[derive(Clone, Copy)]
    pub struct Deadline { pub wait: Wait }
    impl Deadline { pub fn new(wait: Wait) -> Self { Self { wait } } }
}

pub mod session {
    use env::Mark;
    #[derive(Clone, Copy)]
    pub struct Berth { pub link: Mark, pub ask: Mark }
    pub mod establish {
        use env::{Mark, PieToken, TaskId};
        pub fn opened_by(_: PieToken) -> Option<TaskId> { Some(TaskId::new(7)) }
        pub fn find(_: TaskId, _: Mark) -> Option<PieToken> { Some(PieToken::from_bytes(&10u64.to_le_bytes()).unwrap()) }
    }
}

pub mod common {
    pub mod path {
        pub struct Path;
        static PATH: Path = Path;
        impl Path { pub const fn new(_: &'static str) -> &'static Self { &PATH } }
        pub use system_api::operator::path::PathBuf;
    }
}

pub mod system {
    pub mod control {
        pub use crate::control::{Fail, frame, marks, publication};
        pub mod account {
            pub use system_api::control::account::*;
            pub use system_api::control::marks::ACCOUNT_BACK as BACK;
        }
        pub use crate::control::client;
    }
    pub use crate::{identity, loader, operator};
}

#[path = "../../../crates/system-client/src/control/publication.rs"]
pub mod publication_client;

#[macro_export]
macro_rules! debug { ($($arg:tt)*) => {{ let _ = core::format_args!($($arg)*); }}; }

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use system::control::{client::Face, frame::{self, Req, Request, Wire}};

    fn token(value: u64) -> PieToken { PieToken::from_bytes(&value.to_le_bytes()).unwrap() }

    #[test]
    fn control_client_builds_typed_request_and_preserves_wait_budget() {
        CAPTURE.with(|capture| *capture.borrow_mut() = None);
        let face = Face::of(token(9)).unwrap();
        assert_eq!(face.host(), TaskId::new(7));
        assert_eq!(face.service(String::from("alpha")).state(Wait::AtMost(15)), Err(frame::Fail::Bad));
        let (bytes, wait, expected_back) = CAPTURE.with(|capture| capture.borrow_mut().take().unwrap());
        assert_eq!(wait, Wait::AtMost(15));
        assert_eq!(Request::fetch(&bytes), Some((Some(Wire::State(String::from("alpha"))), expected_back)));
    }

    #[test]
    fn typed_contracts_extract_return_tokens_for_all_control_faces() {
        let back = token(0x1234);
        let request = Request(Req::State(String::from("svc")), back);
        let mut bytes = Request::EMPTY;
        let n = request.store(&mut bytes).unwrap();
        let decoded = Request::fetch(&bytes[..n]).unwrap();
        assert_eq!(system_api::control::Call::back(&decoded), back);
        assert_eq!(system_api::control::Call::BACK, frame::BACK);

        let account = system_api::control::account::Request { account: String::from("."), back };
        let mut bytes = system_api::control::account::Request::EMPTY;
        let n = account.store(&mut bytes).unwrap();
        let decoded = system_api::control::account::Request::fetch(&bytes[..n]).unwrap();
        assert_eq!(system_api::control::account::Call::back(&decoded), back);
        assert!(decoded.1, "well-formed trailing boundary is retained for server validation");
        assert_eq!(
            system_api::control::account::Call::BACK,
            system_api::control::marks::ACCOUNT_BACK,
        );
        let decoded_with_tail = system_api::control::account::Request::fetch(&bytes[..n + 1]).unwrap();
        assert!(!decoded_with_tail.1);
        assert_eq!(system_api::control::account::Call::back(&decoded_with_tail), back);

        let frame = system_api::control::publication::Frame::new(
            system_api::control::publication::PUBLISH,
            system_api::control::publication::Target::Service {
                scope: system_api::control::publication::Scope::Driver,
                group: String::from("group"),
                name: String::from("name"),
            },
            (token(77), system_api::operator::Permit::Public),
        );
        let mut bytes = system_api::control::publication::Frame::EMPTY;
        let n = frame.store(&mut bytes).unwrap();
        let decoded = system_api::control::publication::Frame::fetch(&bytes[..n]).unwrap();
        assert_eq!(system_api::control::publication::Call::back(&decoded), frame.back);
        assert_eq!(
            system_api::control::publication::Call::BACK,
            system_api::control::marks::PUBLICATION_BACK,
        );
    }

    fn publication_client(fail_send: bool, fail_receive: bool) -> publication_client::Client {
        PUBLICATION.with(|state| {
            let mut state = state.borrow_mut();
            state.fail_send = fail_send;
            state.fail_receive = fail_receive;
            state.seed = None;
            state.revoked.clear();
        });
        publication_client::Client::direct(TaskId::new(7), token(10)).unwrap()
    }

    fn publication_target() -> system_api::control::publication::Target {
        system_api::control::publication::Target::Service {
            scope: system_api::control::publication::Scope::Driver,
            group: String::from("group"),
            name: String::from("service"),
        }
    }

    #[test]
    fn publication_revokes_seed_when_request_send_fails_before_admission() {
        let client = publication_client(true, false);
        assert!(client.publish(
            publication_target(), token(20), system_api::operator::Permit::Public, Wait::AtMost(12),
        ).is_err());
        let (seed, revoked) = PUBLICATION.with(|state| {
            let state = state.borrow();
            (state.seed.unwrap(), state.revoked.clone())
        });
        assert_eq!(revoked, [(TaskId::new(7), seed)]);
    }

    #[test]
    fn publication_keeps_seed_when_reply_receive_fails_after_admission() {
        let client = publication_client(false, true);
        assert!(client.publish(
            publication_target(), token(20), system_api::operator::Permit::Public, Wait::AtMost(12),
        ).is_err());
        let (seed, revoked) = PUBLICATION.with(|state| {
            let state = state.borrow();
            (state.seed.unwrap(), state.revoked.clone())
        });
        assert!(seed.get() != 0);
        assert!(revoked.is_empty());
    }
}
