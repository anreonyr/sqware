use core::marker::PhantomData;

pub struct Sender<T>(pub PhantomData<T>);

impl Sender<system_api::control::frame::Said> {
    pub fn send(self, said: system_api::control::frame::Said) -> Result<(), ()> {
        crate::ANSWERS.with(|answers| answers.borrow_mut().push(said));
        Ok(())
    }
}
