use core::marker::PhantomData;

pub struct Pending<C>(PhantomData<C>);

impl<C> Pending<C> {
    pub fn new() -> Self {
        Self(PhantomData)
    }

    pub fn poll(&mut self) -> Result<Option<crate::system_api::retirement::Retirement>, ()> {
        crate::backend::poll()
    }
}
