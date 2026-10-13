use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::manager::frame::Frame;
use crate::core::memory::manager::table::TableNode;

use super::index::Index;
use super::invalidation::Invalidation;
use super::map::Map;
use super::phase::ClosePhase;

pub(super) struct TreeState {
    pub(super) root: TableNode,
    pub(super) index: Index,
    pub(super) retired_frames: Vec<Arc<Frame>>,
    pub(super) retired_tables: Vec<TableNode>,
    pub(super) invalidation: Invalidation,
    pub(super) pending_maps: Vec<Arc<Map>>,
    pub(super) phase: ClosePhase,
}

impl TreeState {
    /// 新树：根页表已建，无 Map、无待失效资源。
    pub(super) fn new(root: TableNode) -> Self {
        Self {
            root,
            index: Index::new(),
            retired_frames: Vec::new(),
            retired_tables: Vec::new(),
            invalidation: Invalidation::Clean,
            pending_maps: Vec::new(),
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

    /// shootdown 成功后的唯一结算点：失效清 Clean，关闭中的树进入 Closed。
    /// 退休资源仍留在原字段，由 Tree 取出并在锁外释放。
    pub(super) fn settle(&mut self) {
        self.invalidation = self.invalidation.complete();
        self.phase = self.phase.settle();
    }

    /// 映射已全部撤下：等待 shootdown 结算。
    pub(super) fn begin_close(&mut self) {
        self.phase = self.phase.begin();
    }
}
