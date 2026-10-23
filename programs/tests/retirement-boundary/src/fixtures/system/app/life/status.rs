use core::sync::atomic::{AtomicU8, AtomicUsize};

pub struct Status {
    pub operator: AtomicUsize,
    pub identity: AtomicUsize,
    pub phase: AtomicU8,
}
