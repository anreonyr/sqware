use alloc::rc::Rc;
use core::cell::RefCell;
use core::marker::PhantomData;

use env::{PieToken, Wait};

use super::script::Script;

pub trait Fixture {
    type Request;
    type Response;

    fn call(script: &mut Script, request: Self::Request) -> Result<Self::Response, &'static str>;
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
    pub fn call(
        &self,
        _: Wait,
        build: impl FnOnce(PieToken) -> C::Request,
    ) -> Result<C::Response, &'static str> {
        let request = build(PieToken::mint(1));
        C::call(&mut self.shared.borrow_mut(), request)
    }
}
