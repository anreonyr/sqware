//! 一段**独立持有**：物理保留与它在来源记录上的显式登记一起生、一起灭。
//!
//! 它是**普通任务引用**那一档的 RAII：随持有它的引用一起生死，任务退场就跟着撤，
//! 所以它只说明“这一段此刻仍被独立保留”，不等于文档里那份“设备仍可能访问”的
//! 外部物理持有（那一档要跨控制者死亡仍保留，不在本批）。
//! 来源记录只拿它当“这一段不空闲”的显式事实：不靠 `is_pinned`、不数 `Arc`、
//! 也不靠 `Weak` 生存去猜它结清了没有。登记与物理保留在同一个 `commit` 段里落定，
//! 外面看不到两者之间的空档；撤销时先结束物理保留再撤登记，中间只多挡不少挡。

use alloc::sync::Arc;

use crate::core::memory::flat::{FlatRange, Residency};
use env::TaskId;

use super::geometry::Geometry;
use super::source_authorization::SourceAuthorization;

pub(crate) struct IndependentHold {
    /// 物理保留；`Drop` 里先显式放下它，再撤登记。
    residency: Option<Residency>,
    source: Arc<SourceAuthorization>,
    window: Arc<Geometry>,
}

impl IndependentHold {
    /// 收下这一份独立持有：登记位已由调用方在同一 `commit` 段备好，这一步只写。
    /// `holder` 是**当场拿到这一段的那一个任务**：只有它自己能用这一段建映射。
    pub(crate) fn new(
        source: &Arc<SourceAuthorization>,
        window: Arc<Geometry>,
        residency: Residency,
        holder: TaskId,
    ) -> Self {
        source.register_retention(window.clone(), holder);
        Self {
            residency: Some(residency),
            source: source.clone(),
            window,
        }
    }

    /// 这一段当前能用的来源范围（物理保留自己强持）。
    pub(crate) fn range(&self) -> &FlatRange {
        self.residency
            .as_ref()
            .expect("an independent hold keeps its range until it ends")
            .range()
    }

    /// 这一份登记在来源记录上的窗口（自检读口：两边是不是同一个 `Arc`）。
    pub(crate) fn window(&self) -> &Arc<Geometry> {
        &self.window
    }
}

impl Drop for IndependentHold {
    fn drop(&mut self) {
        // 先结束物理保留，再撤登记：中间只可能多挡一会儿，不会少挡。
        drop(self.residency.take());
        self.source.release_retention(&self.window);
    }
}
