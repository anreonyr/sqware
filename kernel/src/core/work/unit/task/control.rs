use alloc::sync::Arc;
use env::ViewControl;

use crate::core::memory::nest::NestError;
use crate::core::work::unit::space::Space;

use super::Task;

impl Task {
    pub(crate) fn track_control(&self, space: &Arc<Space>, id: usize) -> Result<(), NestError> {
        let mut controls = self.controls.lock();
        controls
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        controls.push((Arc::downgrade(space), id));
        Ok(())
    }

    /// 所有 Nest 共用单调、不复用的关系号；id 唯一标识这一笔控制。
    pub(crate) fn release_control(&self, id: usize) {
        self.controls.lock().retain(|(_, held)| *held != id);
    }

    #[cfg(debug_assertions)]
    pub(crate) fn holds_control(&self, id: usize) -> bool {
        self.controls.lock().iter().any(|(_, held)| *held == id)
    }

    /// 任务退场时收掉它控制的每一段关系；未结的登记原地留在本表里等既有重试。
    ///
    /// 返回 `Err` 时表里仍有未结的控制：调用方（`Task::retire` / HUSKS）必须保留
    /// 这个 Task 并再次调用，不能把仍有作用的控制当成已经结清。
    /// `Space` 已消失的那一档是 raw last-drop 的既有边界：没有 Nest 可重试，也不能
    /// 声称结清；它与“已证明结清”分开，只撤掉无法再走的控制记录。
    pub(crate) fn close_controls(&self) -> Result<(), NestError> {
        let mut at = 0;
        let mut failure: Option<NestError> = None;
        loop {
            let Some((space, id)) = self
                .controls
                .lock()
                .get(at)
                .map(|(space, id)| (space.clone(), *id))
            else {
                break;
            };
            let Some(space) = space.upgrade() else {
                self.release_control(id);
                continue;
            };
            // 一把 registry 锁取任意登记：中间提交的槽不会在两次查询里丢掉。
            let Some(entry) = space.views.registered(id) else {
                self.release_control(id);
                continue;
            };
            let Some(control) = entry.control.as_ref() else {
                self.release_control(id);
                continue;
            };
            let (controller, _) = control.control();
            if controller != self.ident.id || control.scope() != ViewControl::Caller {
                self.release_control(id);
                continue;
            }
            let retry = if space.nest.settled(id) {
                Ok(())
            } else {
                if let Some(view) = entry.view() {
                    let _ = space.dispose(view.clone());
                }
                space.nest.retry_invalidation()
            };
            if !space.nest.settled(id) {
                failure = failure.or(retry.err()).or(Some(NestError::Busy));
                at += 1;
            } else {
                // 结清只有一条规则：Registry 项与源侧成员一起撤，controller 也跟着放。
                space.retire_view(id);
            }
            // 沿原行为释放这个空间里其他已结清的 controller（不在本表锁下）。
            space.reap_views();
        }
        // 并发删除可能让游标跳过一项；表非空就不能当成已全部结清。
        if self.controls.lock().is_empty() {
            Ok(())
        } else {
            Err(failure.unwrap_or(NestError::Busy))
        }
    }
}
