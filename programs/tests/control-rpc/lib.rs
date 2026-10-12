#![allow(dead_code)]
extern crate alloc;
extern crate env as abi_env;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

pub use abi_env::{Mark, PieToken, Reason, TaskId, Wait};
use core::marker::PhantomData;
use std::{cell::RefCell, vec::Vec};
use wire::message::Message;

thread_local! {
    static CAPTURE: RefCell<Option<(Vec<u8>, Wait, PieToken)>> = const { RefCell::new(None) };
    static RESPONSE: RefCell<Option<Result<Vec<u8>, rpc::Fail>>> = const { RefCell::new(None) };
}

pub mod unit {
    use crate::TaskId;
    pub fn self_id() -> TaskId {
        TaskId::new(1)
    }
    pub fn sire() -> TaskId {
        TaskId::new(7)
    }
}

pub mod control {
    pub use system_api::control::{
        Fail, Grant, Req, Request, Said, State, Wire, frame, marks,
    };
    pub mod account {
        pub use account_api::*;
    }
    pub mod client {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/system/client/src/control/client.rs"
        ));
    }
}

pub mod identity {
    pub use system_api::identity::{CoalitionId, PrincipalId};
}

pub mod loader {
    use crate::TaskId;
    pub mod frame {
        pub use system_api::loader::Said;
    }
    pub struct Built {
        pub task: TaskId,
        pub team: abi_env::TeamId,
    }
}

pub mod operator {
    pub use system_api::operator::{
        EntryId, Fail, Permit,
        path::{Path, PathBuf},
    };
    pub mod client {
        pub struct Face;
        pub struct Root;
        pub struct Tile;
        impl Face {
            pub fn root(&self) -> Root {
                Root
            }
        }
        impl Root {
            pub fn tile(
                &self,
                _: &system_api::operator::path::PathBuf,
                _: env::Wait,
            ) -> Result<Tile, system_api::operator::Fail> {
                Err(system_api::operator::Fail::Unknown)
            }
        }
        impl Tile {
            pub fn token(&self, _: env::Wait) -> Result<env::PieToken, system_api::operator::Fail> {
                Err(system_api::operator::Fail::Unknown)
            }
        }
    }
    pub use client::Face;
}

pub mod debug {
    pub fn put(_: &str) {}
}
pub mod rpc {
    use super::*;

    #[derive(Debug)]
    pub enum Fail {
        Open(()),
        Grant(()),
        Encode,
        Decode,
        Send(()),
        Receive(()),
        Untrusted,
        WrongSource,
    }

    pub mod request {
        use super::*;

        pub struct Sender<C: wire::Contract> {
            entry: PieToken,
            _contract: PhantomData<C>,
        }
        impl<C: wire::Contract> Sender<C> {
            pub fn from_raw(entry: PieToken, _: Mark) -> Result<Self, Fail> {
                Ok(Self {
                    entry,
                    _contract: PhantomData,
                })
            }
            pub fn peer(&self) -> TaskId {
                TaskId::new(7)
            }
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
                match RESPONSE.with(|response| response.borrow_mut().take()) {
                    Some(Ok(bytes)) => C::Response::fetch(&bytes).ok_or(Fail::Decode),
                    Some(Err(fail)) => Err(fail),
                    None => Err(Fail::Send(())),
                }
            }

        }
    }
}

pub mod time {
    use env::Wait;
    #[derive(Clone, Copy)]
    pub struct Deadline {
        pub wait: Wait,
    }
    impl Deadline {
        pub fn new(wait: Wait) -> Self {
            Self { wait }
        }
    }
}

pub mod session {
    use env::Mark;
    #[derive(Clone, Copy)]
    pub struct Berth {
        pub link: Mark,
        pub ask: Mark,
    }
    pub mod establish {
        use env::{Mark, PieToken, TaskId};
        pub fn opened_by(_: PieToken) -> Option<TaskId> {
            Some(TaskId::new(7))
        }
        pub fn find(_: TaskId, _: Mark) -> Result<PieToken, DiscoveryFail> {
            Ok(PieToken::from_bytes(&10u64.to_le_bytes()).unwrap())
        }
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum DiscoveryFail {
            Missing,
            Ambiguous,
        }
    }
}

pub mod common {
    pub mod path {
        pub struct Path;
        static PATH: Path = Path;
        impl Path {
            pub const fn new(_: &'static str) -> &'static Self {
                &PATH
            }
        }
        pub use system_api::operator::path::PathBuf;
    }
}

pub mod system {
    pub mod control {
        pub use crate::control::{Fail, frame, marks};
        pub mod account {
            pub use account_api::BACK;
            pub use account_api::*;
        }
        pub use crate::control::client;
    }
    pub use crate::{identity, loader, operator};
}

#[macro_export]
macro_rules! debug { ($($arg:tt)*) => {{ let _ = core::format_args!($($arg)*); }}; }

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use system::control::{
        client::Face,
        frame::{self, Req, Request, Wire},
    };

    fn token(value: u64) -> PieToken {
        PieToken::from_bytes(&value.to_le_bytes()).unwrap()
    }

    #[test]
    fn control_client_builds_typed_request_and_preserves_wait_budget() {
        CAPTURE.with(|capture| *capture.borrow_mut() = None);
        let face = Face::of(token(9)).unwrap();
        assert_eq!(face.host(), TaskId::new(7));
        assert_eq!(
            face.service(String::from("alpha")).state(Wait::AtMost(15)),
            Err(frame::Fail::Bad)
        );
        let (bytes, wait, expected_back) =
            CAPTURE.with(|capture| capture.borrow_mut().take().unwrap());
        assert_eq!(wait, Wait::AtMost(15));
        assert_eq!(
            Request::fetch(&bytes),
            Some((Some(Wire::State(String::from("alpha"))), expected_back))
        );
    }

    fn said(status: u8, state: u8, task: usize) -> Vec<u8> {
        use wire::Message;
        let value = system_api::control::Said { status, a: state, task: TaskId::new(task), reason: 0, completed: false };
        let mut bytes = vec![0; system_api::control::Said::LEN];
        value.store(&mut bytes).unwrap(); bytes
    }
    #[test]
    fn named_task_query_preserves_budget_and_rejects_invalid_or_wrong_source_answers() {
        let face = Face::of(token(9)).unwrap();
        RESPONSE.with(|response| {
            *response.borrow_mut() = Some(Ok(said(0, 0, 22)))
        });
        assert_eq!(
            face.task(String::from("login"), Wait::AtMost(19)),
            Ok(TaskId::new(22))
        );
        let (bytes, wait, back) = CAPTURE.with(|capture| capture.borrow_mut().take().unwrap());
        assert_eq!(wait, Wait::AtMost(19));
        assert_eq!(
            Request::fetch(&bytes),
            Some((Some(Wire::Task(String::from("login"))), back))
        );
        for bytes in [
            said(0, 0, 0),
            said(0, 1, 22),
            vec![0; 9],
            vec![0; 11],
        ] {
            RESPONSE.with(|response| *response.borrow_mut() = Some(Ok(bytes)));
            assert_eq!(
                face.task(String::from("login"), Wait::POLL),
                Err(frame::Fail::Bad)
            );
        }
        RESPONSE.with(|response| *response.borrow_mut() = Some(Err(rpc::Fail::WrongSource)));
        assert_eq!(
            face.task(String::from("login"), Wait::POLL),
            Err(frame::Fail::Bad)
        );
        for status in [1, 4] {
            RESPONSE.with(|response| {
                *response.borrow_mut() = Some(Ok(said(status, 0, 0)))
            });
            assert_eq!(
                face.task(String::from("login"), Wait::POLL),
                Err(if status == 1 {
                    frame::Fail::Unknown
                } else {
                    frame::Fail::NotReady
                })
            );
        }
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

        let account = account_api::Request {
            op: account_api::CREATE,
            id: system_api::control::construction::CreateId { value: 1 },
            account: String::from("."),
            back,
        };
        let mut bytes = account_api::Request::EMPTY;
        let n = account.store(&mut bytes).unwrap();
        let decoded = account_api::Request::fetch(&bytes[..n]).unwrap();
        assert_eq!(account_api::Call::back(&decoded), back);
        assert!(
            decoded.1,
            "well-formed trailing boundary is retained for server validation"
        );
        assert_eq!(account_api::Call::BACK, account_api::BACK,);
        let decoded_with_tail = account_api::Request::fetch(&bytes[..n + 1]).unwrap();
        assert!(!decoded_with_tail.1);
        assert_eq!(account_api::Call::back(&decoded_with_tail), back);

        let export = system_api::identity::export::Request {
            object: system_api::identity::IdentityObject::Principal(system_api::identity::PrincipalId::new(TaskId::new(7), 1)),
            alias: system_api::identity::IdentityName::new("anran").unwrap(),
            binding: system_api::identity::IdentityName::new("create").unwrap(),
            entry: system_api::identity::OperatorEntryRef { operator: TaskId::new(9), root: false, id: 0, revision: 1 },
            back,
        };
        let mut bytes = system_api::identity::export::Request::EMPTY;
        let n = export.store(&mut bytes).unwrap();
        let decoded = system_api::identity::export::Request::fetch(&bytes[..n]).unwrap();
        assert_eq!(system_api::identity::export::Call::back(&decoded), back);
        assert_eq!(system_api::identity::export::Call::BACK, system_api::identity::export::BACK);
    }
}
