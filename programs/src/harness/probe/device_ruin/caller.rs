use alloc::sync::Arc;
use core::sync::atomic::AtomicBool;

pub(super) struct Caller {
    pub task: env::TaskId,
    pub ready: Arc<AtomicBool>,
}
