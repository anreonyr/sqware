use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) static FRAME_TAKE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static FRAME_GIVE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static BLOCK_TAKE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static BLOCK_GIVE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static POOL_TAKE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static POOL_GIVE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static SPARE_TAKE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static SPARE_GIVE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static SPARE_TOTAL: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn record_spare_total(total: usize) {
    SPARE_TOTAL.store(total, Ordering::Relaxed);
}
