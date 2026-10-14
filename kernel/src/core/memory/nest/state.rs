use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::flat::Frame;
use crate::core::memory::nest::View;
use crate::core::memory::nest::synchronize::{ClosePhase, Invalidation};
use crate::core::memory::nest::table::TableNode;

use super::index::Index;

/// 地址空间里除每关系库存外的全部状态：根页表、关系索引、
/// 待失效标记、退休中的资源与尚未结算的占用票。
pub(super) struct NestState {
    pub(super) root: TableNode,
    pub(super) index: Index,
    pub(super) retired_frames: Vec<Arc<Frame>>,
    pub(super) retired_tables: Vec<TableNode>,
    pub(super) invalidation: Invalidation,
    pub(super) pending_views: Vec<Arc<View>>,
    /// 已退休、尚未结算的占用票：同步失败时留在这里，VA 禁复用。
    pub(super) retirement: Vec<usize>,
    pub(super) phase: ClosePhase,
}

impl NestState {
    /// 新地址空间：根页表已建，无关系、无待失效资源。
    pub(super) fn new(root: TableNode) -> Self {
        Self {
            root,
            index: Index::new(),
            retired_frames: Vec::new(),
            retired_tables: Vec::new(),
            invalidation: Invalidation::Clean,
            pending_views: Vec::new(),
            retirement: Vec::new(),
            phase: ClosePhase::Open,
        }
    }

    /// PTE 已修改：留待 shootdown。
    pub(super) fn mark_dirty(&mut self) {
        self.invalidation = self.invalidation.mark_dirty();
    }

    /// 开始（或重试）shootdown。
    pub(super) fn fence(&mut self) {
        self.invalidation = self.invalidation.begin();
    }

    /// shootdown 失败：保留待失效标记与退休资源。
    pub(super) fn fail(&mut self) {
        self.invalidation = self.invalidation.fail();
    }

    /// shootdown 成功后的唯一结算点：失效清 Clean，关闭中的空间进入 Closed。
    /// 退休资源仍留在原字段，由 Nest 取出并在锁外释放。
    pub(super) fn settle(&mut self) {
        self.invalidation = self.invalidation.complete();
        self.phase = self.phase.settle();
    }

    /// 关系已全部撤下：等待 shootdown 结算。
    pub(super) fn begin_close(&mut self) {
        self.phase = self.phase.begin();
    }
}
