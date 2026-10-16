use alloc::sync::Arc;

use crate::core::memory::nest::{NestError, Reservation, View};
use crate::core::work::unit::space::Space;

/// 任务自己的内存段：空间、栈（含守卫的保留格）与外壳页。字段由
/// `TaskBuilder::prepare` 按申请—映射—回滚的次序装配。
pub(crate) struct TaskMemory {
    pub(crate) space: Arc<Space>,
    pub(crate) stack: Option<Arc<View>>,
    pub(crate) frame: Option<Arc<View>>,
    pub(crate) guard: Option<Reservation>,
}

impl TaskMemory {
    /// 按段撤下：失败的那一段留在原地，沿用同一身份可重试。
    pub(super) fn retire(&mut self) -> Result<(), NestError> {
        if let Some(view) = &self.frame {
            self.space.withdraw(view.whole())?;
            self.frame = None;
        }
        if let Some(view) = &self.stack {
            self.space.withdraw(view.whole())?;
            self.stack = None;
        }
        if let Some(guard) = self.guard.take() {
            self.space.nest.release_reservation(guard)?;
        }
        Ok(())
    }
}

impl Drop for TaskMemory {
    fn drop(&mut self) {
        if let Some(view) = self.frame.take() {
            let _ = self.space.dispose(view);
        }
        if let Some(view) = self.stack.take() {
            let _ = self.space.dispose(view);
        }
        if let Some(guard) = self.guard.take() {
            let _ = self.space.nest.release_reservation(guard);
        }
    }
}
