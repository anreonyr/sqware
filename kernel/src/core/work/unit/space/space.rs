use alloc::sync::{Arc, Weak};

use crate::core::memory::manager::{Memory, SpaceKind};
use crate::core::work::unit::life::Life;

/// 一个团队的地址空间：内存状态（`Memory`）加一条生命周期线。
pub struct Space {
    pub(crate) memory: Memory,
    pub(super) life: Arc<Life>,
}

impl Space {
    pub fn kind(&self) -> SpaceKind {
        self.memory.kind()
    }

    pub fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }
}

unsafe impl Send for Space {}
unsafe impl Sync for Space {}
