use alloc::sync::Arc;
use alloc::vec::Vec;

use env::TaskId;

pub(super) struct Hand {
    pub(super) from: TaskId,
    pub(super) buf: Arc<Vec<u8>>,
    pub(super) at: u64,
}
