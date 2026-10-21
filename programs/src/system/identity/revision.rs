//! Revision and notification shared by the authority and its supervisor.
use ::resource::bell::Bell;
use ::schedule::{Progress, Res};
use alloc::sync::Arc;
use core::sync::atomic::AtomicU64;
use env::{NoleFail, Wait};

#[derive(Clone)]
pub struct Epoch(pub Arc<AtomicU64>);
pub struct Changed(pub Bell);
impl Epoch {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }
}

pub(crate) fn consume(changed: Res<Changed>) -> Result<Progress, &'static str> {
    match changed.0.wait(Wait::POLL) {
        Ok(true) => match changed.0.hush() {
            Ok(()) => Ok(Progress::Done),
            Err(error) if error.source == NoleFail::Busy => Ok(Progress::Done),
            Err(_) => Err("identity revision bell"),
        },
        Ok(false) => Ok(Progress::Done),
        Err(error) if error.source == NoleFail::Busy => Ok(Progress::Done),
        Err(_) => Err("identity revision bell"),
    }
}
