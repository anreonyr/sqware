use ::core::marker::PhantomData;
use system_api::control::{Fail, frame::Said};

pub struct Pending<C> {
    kind: crate::backend::Kind,
    _contract: PhantomData<C>,
}

impl<C> Pending<C> {
    pub(crate) fn new(kind: crate::backend::Kind) -> Self {
        Self {
            kind,
            _contract: PhantomData,
        }
    }

    pub fn poll(&mut self) -> Result<Option<Said>, Fail> {
        crate::backend::poll(self.kind)
    }
}
