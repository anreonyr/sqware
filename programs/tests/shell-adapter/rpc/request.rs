use ::core::marker::PhantomData;

pub trait Reply {
    type In;
    type Fail;
    fn poll() -> Result<Option<Self::In>, Self::Fail>;
}

impl Reply for pipe_api::Call {
    type In = pipe_api::Reply;
    type Fail = pipe_api::Fail;
    fn poll() -> Result<Option<Self::In>, Self::Fail> {
        Ok(None)
    }
}

impl Reply for system_api::control::Call {
    type In = system_api::control::frame::Said;
    type Fail = system_api::control::Fail;
    fn poll() -> Result<Option<Self::In>, Self::Fail> {
        crate::backend::poll_release()
    }
}

pub struct Pending<C: Reply> {
    _contract: PhantomData<C>,
}

impl<C: Reply> Pending<C> {
    pub(crate) fn new() -> Self {
        Self {
            _contract: PhantomData,
        }
    }

    pub fn poll(&mut self) -> Result<Option<C::In>, C::Fail> {
        C::poll()
    }

    pub fn source(&self) -> crate::ToleEventSource {
        crate::ToleEventSource::Join {
            target: crate::unit::self_id().into(),
        }
    }

    pub fn remaining(&self) -> crate::Wait {
        crate::Wait::AtMost(1)
    }
}
