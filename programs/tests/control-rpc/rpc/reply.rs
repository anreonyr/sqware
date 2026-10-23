use core::{cell::Cell, marker::PhantomData};

thread_local! {
    static SENT: Cell<usize> = const { Cell::new(0) };
}

pub struct Sender<R> {
    accepted: bool,
    _response: PhantomData<R>,
}

impl<R> Sender<R> {
    pub fn new(accepted: bool) -> Self {
        Self { accepted, _response: PhantomData }
    }
    pub fn send(self, _: R) -> Result<(), ()> {
        SENT.with(|sent| sent.set(sent.get() + 1));
        if self.accepted { Ok(()) } else { Err(()) }
    }
}

pub fn reset() { SENT.with(|sent| sent.set(0)); }
pub fn sent() -> usize { SENT.with(Cell::get) }
