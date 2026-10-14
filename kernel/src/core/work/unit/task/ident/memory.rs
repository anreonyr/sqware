use alloc::sync::Arc;

use crate::core::memory::nest::{NestError, Reservation};
use crate::core::work::unit::space::{Mapping, Space};

/// 任务自己的内存段：空间、栈（含守卫的保留格）与外壳页。字段由
/// `TaskBuilder::prepare` 按申请—映射—回滚的次序装配。
pub(crate) struct TaskMemory {
    pub(crate) space: Arc<Space>,
    pub(crate) stack: Option<Mapping>,
    pub(crate) frame: Option<Mapping>,
    pub(crate) guard: Option<Reservation>,
}

impl TaskMemory {
    pub(super) fn retire(&mut self) -> Result<(), NestError> {
        if let Some(mapping) = &self.frame {
            self.space.memory.unmap(mapping)?;
            self.frame = None;
        }
        if let Some(mapping) = &self.stack {
            self.space.memory.unmap(mapping)?;
            self.stack = None;
        }
        if let Some(guard) = self.guard.take() {
            self.space.memory.release_reservation(guard)?;
        }
        Ok(())
    }
}

impl Drop for TaskMemory {
    fn drop(&mut self) {
        if let Some(mapping) = self.frame.take() {
            let _ = self.space.memory.dispose(mapping);
        }
        if let Some(mapping) = self.stack.take() {
            let _ = self.space.memory.dispose(mapping);
        }
        if let Some(guard) = self.guard.take() {
            let _ = self.space.memory.release_reservation(guard);
        }
    }
}
