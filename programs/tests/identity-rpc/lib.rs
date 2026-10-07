extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

pub use abi::{
    Mark, PieFail, PieResult, PieToken, TaskId, Wait, make_fail,
};

pub mod raw {
    use crate::{Mark, PieFail, PieResult, PieToken, TaskId, make_fail, test_state};

    pub fn reserve(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
        test_state::reserve(token).ok_or_else(|| make_fail(PieFail::Denied))
    }
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
    use wire::Message;

    pub trait Contract {
        type Request: Message;
        type Response: Message<In = system_api::identity::Reply>;
        const BACK: crate::Mark;
        fn back(request: &<Self::Request as Message>::In) -> PieToken;
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Fail { Encode, Decode, WrongSource, Other }

    pub mod request {
        use super::*;

        pub struct Sender<C: Contract> { entry: PieToken, _contract: PhantomData<fn() -> C> }

        impl<C: Contract> Sender<C> {
            pub fn from_raw(entry: PieToken) -> Result<Self, Fail> {
                Ok(Self { entry, _contract: PhantomData })
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
                let response = crate::test_state::rpc_result()?;
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

pub mod system;

pub mod test_state {
    use std::{cell::RefCell, collections::HashMap};
    use abi::{Mark, PieToken, TaskId, Wait};
    use system_api::identity::Reply;
    use crate::rpc::Fail;

    #[derive(Clone, Copy)]
    struct Record { vestor: TaskId, owner: TaskId, mark: Mark }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Sent { pub entry: PieToken, pub built_back: PieToken, pub extracted_back: PieToken, pub wait: Wait }
    #[derive(Default)]
    struct State {
        records: HashMap<PieToken, Record>,
        next_back: usize,
        sent: Vec<Sent>,
        response: Option<Result<Reply, Fail>>,
    }
    thread_local! { static STATE: RefCell<State> = RefCell::new(State { next_back: 800, ..State::default() }); }

    pub fn reset() { STATE.with(|s| *s.borrow_mut() = State { next_back: 800, ..State::default() }); }
    pub fn authorize(entry: PieToken, vestor: TaskId, owner: TaskId, mark: Mark) {
        STATE.with(|s| { s.borrow_mut().records.insert(entry, Record { vestor, owner, mark }); });
    }
    pub fn reserve(entry: PieToken) -> Option<(TaskId, TaskId, Mark)> {
        STATE.with(|s| s.borrow().records.get(&entry).map(|r| (r.vestor, r.owner, r.mark)))
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
