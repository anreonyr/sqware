use ::core::marker::PhantomData;
use system_api::control::{Fail, frame::Said};

pub struct Pending<C> {
    kind: crate::backend::Kind,
    serial: u64,
    _contract: PhantomData<C>,
}

impl Pending<system_api::control::Call> {
    pub(crate) fn new(kind: crate::backend::Kind) -> Self {
        Self {
            kind,
            serial: 0,
            _contract: PhantomData,
        }
    }

    pub fn poll(&mut self) -> Result<Option<Said>, Fail> {
        crate::backend::poll(self.kind)
    }
}

impl Pending<hub_api::coordination::Call> {
    pub(crate) fn coordinate(request: hub_api::coordination::Wire) -> Self {
        let serial = match request {
            hub_api::coordination::Wire::Prepare(frame) => frame.serial,
            hub_api::coordination::Wire::Release(frame) => frame.serial,
        };
        crate::coord_backend::begin(request);
        Self { kind: crate::backend::Kind::Release, serial, _contract: PhantomData }
    }

    pub fn poll(&mut self) -> Result<Option<hub_api::DmaSaid>, Fail> {
        crate::coord_backend::poll(self.serial)
    }
}
