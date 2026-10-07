//! Revision and notification shared by the authority and its supervisor.
use alloc::sync::Arc;
use core::sync::atomic::AtomicU64;
use ::resource::bell::Bell;

#[derive(Clone)]
pub struct Epoch(pub Arc<AtomicU64>);
pub struct Changed(pub Bell);
impl Epoch {
    pub fn new() -> Self { Self(Arc::new(AtomicU64::new(0))) }
}
