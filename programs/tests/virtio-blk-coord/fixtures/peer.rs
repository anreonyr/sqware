use alloc::rc::Rc;
use core::cell::RefCell;
use core::marker::PhantomData;

use env::{PieToken, Wait};

use super::script::Script;

pub trait Fixture {
    type Request: Clone;
    type Response;

    fn call(script: &mut Script, request: Self::Request) -> Result<Self::Response, &'static str>;

    fn held(script: &Script, request: &Self::Request) -> bool;
}

pub struct Peer<C> {
    shared: Rc<RefCell<Script>>,
    _contract: PhantomData<C>,
}

impl<C> Peer<C> {
    pub fn host(&self) -> env::TaskId { env::TaskId::new(10) }

    pub fn new(shared: Rc<RefCell<Script>>) -> Self {
        Self {
            shared,
            _contract: PhantomData,
        }
    }
}

impl<C: Fixture> Peer<C> {
    pub fn begin(
        &self,
        _: Wait,
        build: impl FnOnce(PieToken) -> C::Request,
    ) -> Result<crate::rpc::request::Pending<C>, &'static str> {
        let request = build(PieToken::mint(1));
        Ok(crate::rpc::request::Pending::new(self.shared.clone(), request))
    }

    pub fn call(
        &self,
        _: Wait,
        build: impl FnOnce(PieToken) -> C::Request,
    ) -> Result<C::Response, &'static str> {
        let request = build(PieToken::mint(1));
        C::call(&mut self.shared.borrow_mut(), request)
    }
}
