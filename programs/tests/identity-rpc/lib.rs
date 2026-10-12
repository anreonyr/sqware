extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

pub use abi::{
    Mark, Permission, PieFail, PieInfo, PieKind, PieResult, PieToken, TaskId, Wait, make_fail,
};

pub mod raw {
    use crate::{Mark, PieFail, PieInfo, PieResult, PieToken, TaskId, make_fail, test_state};

    pub fn reserve(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
        test_state::reserve(token).ok_or_else(|| make_fail(PieFail::Denied))
    }
    pub fn inspect(token: PieToken) -> PieResult<PieInfo> {
        test_state::inspect(token).ok_or_else(|| make_fail(PieFail::Denied))
    }
    pub struct Entry { pub token: PieToken, pub mark: Mark }
    pub fn pies() -> impl Iterator<Item = Entry> {
        test_state::entries().into_iter().map(|(token, mark)| Entry { token, mark })
    }
}

pub mod session {
    pub mod establish {
        pub fn claim(authority: crate::TaskId, mark: crate::Mark, _: crate::Wait) -> Result<crate::PieToken, ()> {
            let mut matches = crate::test_state::entries().into_iter().filter_map(|(token, actual)| {
                (actual == mark && crate::test_state::inspect(token).is_some_and(|info| info.alive && info.owner == authority)).then_some(token)
            });
            let first = matches.next().ok_or(())?;
            if matches.next().is_some() { return Err(()); }
            Ok(first)
        }
    }
}

pub mod pie {
    use crate::{Mark, Permission, PieResult, PieToken, TaskId};
    pub fn accord(_: PieToken, _: TaskId, _: Permission, _: Mark) -> PieResult<PieToken> {
        Ok(PieToken::mint(900))
    }
}

pub mod unit {
    pub fn sire() -> crate::TaskId { crate::test_state::sire() }
}

pub mod time {
    use crate::Wait;
    #[derive(Clone, Copy)]
    pub struct Deadline(pub Wait);
    impl Deadline { pub fn new(wait: Wait) -> Self { Self(wait) } }
}

pub mod rpc {
    use core::marker::PhantomData;
    use crate::{PieToken, time::Deadline};
    use wire::{Contract, Message};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Fail { Encode, Decode, WrongSource, Other }

    pub trait MockContract: Contract {
        fn back(request: &<Self::Request as Message>::In) -> PieToken;
        fn response(reply: system_api::identity::Reply) -> Self::Response;
    }

    impl MockContract for system_api::identity::Call {
        fn back(request: &<Self::Request as Message>::In) -> PieToken {
            system_api::identity::Call::back(request)
        }
        fn response(reply: system_api::identity::Reply) -> Self::Response { reply }
    }

    impl MockContract for system_api::identity::export::Call {
        fn back(request: &<Self::Request as Message>::In) -> PieToken {
            system_api::identity::export::reply_to(request)
        }
        fn response(reply: system_api::identity::Reply) -> Self::Response {
            system_api::identity::export::Response::rejected(match reply {
                system_api::identity::Reply::Unit => system_api::identity::export::OK,
                system_api::identity::Reply::Fail(system_api::identity::Fail::Full) => system_api::identity::export::FULL,
                _ => system_api::identity::export::DENIED,
            })
        }
    }

    pub mod request {
        use super::*;

        pub struct Sender<C: MockContract> {
            entry: PieToken,
            _contract: PhantomData<fn() -> C>,
        }

        impl<C: MockContract> Sender<C> {
            pub fn from_raw(entry: PieToken, _: crate::Mark) -> Result<Self, Fail> {
                Ok(Self { entry, _contract: PhantomData })
            }

            pub fn peer(&self) -> crate::TaskId {
                crate::test_state::inspect(self.entry).map_or(crate::TaskId::new(0), |info| info.owner)
            }

            pub fn call(
                &self,
                deadline: Deadline,
                build: impl FnOnce(PieToken) -> C::Request,
            ) -> Result<<C::Response as Message>::In, Fail> {
                let back = crate::test_state::next_back();
                let request = build(back);
                let mut buffer = C::Request::EMPTY;
                let len = request.store(buffer.as_mut()).ok_or(Fail::Encode)?;
                let bytes = buffer.as_ref().get(..len).ok_or(Fail::Encode)?;
                let decoded = C::Request::fetch(bytes).ok_or(Fail::Decode)?;
                let extracted = C::back(&decoded);
                crate::test_state::sent(self.entry, back, extracted, deadline.0);
                let response = C::response(crate::test_state::rpc_result()?);
                let mut response_buffer = C::Response::EMPTY;
                let response_len = response.store(response_buffer.as_mut()).ok_or(Fail::Encode)?;
                let response_bytes = response_buffer.as_ref().get(..response_len).ok_or(Fail::Encode)?;
                C::Response::fetch(response_bytes).ok_or(Fail::Decode)
            }
        }
    }
}

pub mod common {
    pub mod path {
        #[derive(Clone, Copy)]
        pub struct Path;
        impl Path {
            pub const fn new(_: &str) -> Self { Self }
            pub fn try_join(&self, _: &str) -> Option<Self> { Some(Self) }
        }
    }
}

#[path = "../../src/system/client/src/identity/mod.rs"]
pub mod identity;
pub use system::operator;

pub mod system;

pub mod test_state {
    use std::{cell::RefCell, collections::HashMap};
    use abi::{Mark, PieToken, TaskId, Wait};
    use system_api::identity::Reply;
    use crate::rpc::Fail;

    #[derive(Clone, Copy)]
    struct Record { vestor: TaskId, owner: TaskId, mark: Mark, permission: abi::Permission, kind: abi::PieKind }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Sent { pub entry: PieToken, pub built_back: PieToken, pub extracted_back: PieToken, pub wait: Wait }
    #[derive(Default)]
    struct State {
        records: HashMap<PieToken, Record>,
        next_back: usize,
        sent: Vec<Sent>,
        response: Option<Result<Reply, Fail>>,
        sire: Option<TaskId>,
    }
    thread_local! { static STATE: RefCell<State> = RefCell::new(State { next_back: 800, ..State::default() }); }

    pub fn reset() { STATE.with(|s| *s.borrow_mut() = State { next_back: 800, ..State::default() }); }
    pub fn authorize(entry: PieToken, vestor: TaskId, owner: TaskId, mark: Mark) {
        STATE.with(|s| { s.borrow_mut().records.insert(entry, Record { vestor, owner, mark, permission: abi::Permission::STORE | abi::Permission::VEST, kind: abi::PieKind::Hole }); });
    }
    pub fn authorize_store(entry: PieToken, owner: TaskId, mark: Mark) {
        STATE.with(|s| { s.borrow_mut().records.insert(entry, Record { vestor: owner, owner, mark, permission: abi::Permission::STORE, kind: abi::PieKind::Hole }); });
    }
    pub fn reserve(entry: PieToken) -> Option<(TaskId, TaskId, Mark)> {
        STATE.with(|s| s.borrow().records.get(&entry).map(|r| (r.vestor, r.owner, r.mark)))
    }
    pub fn inspect(token: PieToken) -> Option<abi::PieInfo> {
        STATE.with(|s| s.borrow().records.get(&token).map(|r| abi::PieInfo { token, kind: r.kind, permission: r.permission, owner: r.owner, vestor: r.vestor, mark: r.mark, alive: true }))
    }
    pub fn set_sire(sire: TaskId) { STATE.with(|s| s.borrow_mut().sire = Some(sire)); }
    pub fn sire() -> TaskId { STATE.with(|s| s.borrow().sire.unwrap_or(TaskId::new(2))) }
    pub fn entries() -> Vec<(PieToken, Mark)> {
        STATE.with(|s| s.borrow().records.iter().map(|(token, record)| (*token, record.mark)).collect())
    }
    pub fn next_back() -> PieToken {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let token = PieToken::mint(state.next_back);
            state.next_back += 1;
            token
        })
    }
    pub fn sent(entry: PieToken, built_back: PieToken, extracted_back: PieToken, wait: Wait) {
        STATE.with(|s| s.borrow_mut().sent.push(Sent { entry, built_back, extracted_back, wait }));
    }
    pub fn sent_calls() -> Vec<Sent> { STATE.with(|s| s.borrow().sent.clone()) }
    pub fn respond(response: Result<Reply, Fail>) { STATE.with(|s| s.borrow_mut().response = Some(response)); }
    pub fn rpc_result() -> Result<Reply, Fail> {
        STATE.with(|s| s.borrow_mut().response.take().unwrap_or(Ok(Reply::Unit)))
    }
}

#[cfg(test)]
mod tests;
