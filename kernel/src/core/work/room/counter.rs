//! 松弛计数：只用来出读数，不参与判定。
//!
//! 调度器的 kick/fallback 与末日的四类杀法各记一份，故归 `room` 这一层。

use core::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct Counter(AtomicUsize);

impl Counter {
    pub(crate) const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub(crate) fn bump(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}
