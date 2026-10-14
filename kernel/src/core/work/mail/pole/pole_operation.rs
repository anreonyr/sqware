use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use super::PoleMeta;

pub(crate) struct PoleOperation(Arc<PoleMeta>);

impl PoleOperation {
    pub(crate) fn acquire(meta: &Arc<PoleMeta>) -> Option<Self> {
        meta.operating
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        Some(Self(meta.clone()))
    }
}

impl Drop for PoleOperation {
    fn drop(&mut self) {
        self.0.operating.store(false, Ordering::Release);
    }
}
