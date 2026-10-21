use std::sync::atomic::{AtomicU64, Ordering};

pub static TICKS: AtomicU64 = AtomicU64::new(0);
pub struct Instant(u64);

impl Instant {
    pub fn as_ticks(self) -> u64 {
        self.0
    }
}

pub fn now() -> Instant {
    Instant(TICKS.fetch_add(10_000, Ordering::Relaxed))
}
