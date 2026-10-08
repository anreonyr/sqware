#![allow(dead_code)]
extern crate alloc;
extern crate env as abi_env;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

pub use abi_env::{Mark, Permission, PieToken, TaskId, TeamId, Wait};
use core::cell::RefCell;
use std::vec::Vec;
use system_api::loader::{Said, Wire};
use wire::Message;

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportFail {
    Send,
    Receive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    Build {
        image: PieToken,
        offset: u64,
        len: u64,
        stack: u64,
        count: u8,
        args: [u64; system_api::loader::MAX_ARGS],
        back: PieToken,
    },
    Claim {
        task: TaskId,
        back: PieToken,
    },
}

struct State {
    now_ms: u64,
    calls: Vec<CallRecord>,
    replies: Vec<Result<Said, TransportFail>>,
    events: Vec<Event>,
    fail_grant: bool,
    fail_open: bool,
    elapsed_after_call_ms: u64,
    elapsed_grant_ms: u64,
}
struct CallRecord {
    request: Captured,
    wait: Wait,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Grant(PieToken, TaskId, Permission, Mark, PieToken),
    Revoke(TaskId, PieToken),
}
impl State {
    const fn new() -> Self {
        Self {
            now_ms: 0,
            calls: Vec::new(),
            replies: Vec::new(),
            events: Vec::new(),
            fail_grant: false,
            fail_open: false,
            elapsed_after_call_ms: 0,
            elapsed_grant_ms: 0,
        }
    }
    fn reset(&mut self) {
        *self = Self::new();
    }
}

pub mod chrono {
    use crate::STATE;
    pub fn clock() -> u64 {
        STATE.with(|state| state.borrow().now_ms * 1_000_000)
    }
}

pub mod raw {
    use crate::{Event, Mark, Permission, PieToken, STATE, TaskId};
    use core::marker::PhantomData;
    pub struct Loan<'a> {
        remote: PieToken,
        peer: TaskId,
        _source: PhantomData<&'a PieToken>,
    }
    impl<'a> Loan<'a> {
        pub fn accord(
            source: &'a PieToken,
            peer: TaskId,
            permission: Permission,
            mark: Mark,
        ) -> Result<Self, ()> {
            let remote = PieToken::from_bytes(&0xabcdu64.to_le_bytes()).unwrap();
            let fail = STATE.with(|state| {
                let mut state = state.borrow_mut();
                state
                    .events
                    .push(Event::Grant(*source, peer, permission, mark, remote));
                state.now_ms = state.now_ms.saturating_add(state.elapsed_grant_ms);
                state.fail_grant
            });
            if fail {
                return Err(());
            }
            Ok(Self {
                remote,
                peer,
                _source: PhantomData,
            })
        }
        pub fn remote(&self) -> PieToken {
            self.remote
        }
    }
    impl Drop for Loan<'_> {
        fn drop(&mut self) {
            STATE.with(|state| {
                state
                    .borrow_mut()
                    .events
                    .push(Event::Revoke(self.peer, self.remote))
            });
        }
    }
}

pub mod time {
    use crate::{STATE, Wait};
    #[derive(Clone, Copy)]
    pub struct Deadline {
        start_ms: u64,
        budget: Option<usize>,
    }
    impl Deadline {
        pub fn new(wait: Wait) -> Self {
            let budget = match wait {
                Wait::Forever => None,
                Wait::AtMost(ms) => Some(ms),
            };
            Self {
                start_ms: STATE.with(|state| state.borrow().now_ms),
                budget,
            }
        }
        pub fn remaining(self) -> Wait {
            match self.budget {
                None => Wait::Forever,
                Some(ms) => {
                    let now = STATE.with(|state| state.borrow().now_ms);
                    Wait::AtMost(ms.saturating_sub(now.saturating_sub(self.start_ms) as usize))
                }
            }
        }
    }
}

pub mod rpc {
    use super::*;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Fail;
    pub mod request {
        use super::*;
        pub struct Sender<C: wire::Contract> {
            _entry: PieToken,
            _contract: core::marker::PhantomData<C>,
        }
        impl<C: wire::Contract> Sender<C> {
            pub fn from_raw(entry: PieToken, _: Mark) -> Result<Self, Fail> {
                let fail = STATE.with(|state| state.borrow().fail_open);
                if fail {
                    Err(Fail)
                } else {
                    Ok(Self {
                        _entry: entry,
                        _contract: core::marker::PhantomData,
                    })
                }
            }
            pub fn peer(&self) -> TaskId {
                TaskId::new(7)
            }
            pub fn call(
                &self,
                deadline: crate::time::Deadline,
                build: impl FnOnce(PieToken) -> C::Request,
            ) -> Result<<C::Response as Message>::In, Fail> {
                let back = PieToken::from_bytes(&0x5000u64.to_le_bytes()).unwrap();
                let request = build(back);
                let mut bytes = C::Request::EMPTY;
                let len = request.store(bytes.as_mut()).ok_or(Fail)?;
                let decoded = Wire::fetch(&bytes.as_ref()[..len]).ok_or(Fail)?;
                let captured = match decoded {
                    Wire::Build(ask) => Captured::Build {
                        image: ask.image,
                        offset: ask.offset,
                        len: ask.len,
                        stack: ask.stack,
                        count: ask.count,
                        args: ask.args,
                        back: ask.back,
                    },
                    Wire::Claim(claim) => Captured::Claim {
                        task: claim.task,
                        back: claim.back,
                    },
                };
                let wait = deadline.remaining();
                let reply = STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    state.calls.push(CallRecord {
                        request: captured,
                        wait,
                    });
                    state.now_ms = state.now_ms.saturating_add(state.elapsed_after_call_ms);
                    state.replies.remove(0)
                });
                let said = reply.map_err(|_| Fail)?;
                let mut response = C::Response::EMPTY;
                let len = said.store(response.as_mut()).ok_or(Fail)?;
                C::Response::fetch(&response.as_ref()[..len]).ok_or(Fail)
            }
        }
    }
}

pub mod loader {
    pub mod frame {
        pub use system_api::loader::*;
    }
    pub mod client {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../crates/system-client/src/loader/client.rs"
        ));
    }
    pub use client::Face;
}

#[cfg(test)]
mod tests;
