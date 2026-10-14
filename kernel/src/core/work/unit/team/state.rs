use alloc::vec::Vec;

use env::UnitFail;

use super::Staging;
use super::TeamPhase;
use super::TeamStage;

/// 团队状态机：阶段（`TeamStage`）、相位（`TeamPhase`）与暂存表。
pub(crate) struct TeamState {
    pub(crate) stage: TeamStage,
    pub(crate) phase: TeamPhase,
    pub(crate) staged: Vec<Staging>,
}

unsafe impl Send for TeamState {}

impl TeamState {
    pub(crate) fn new(stage: TeamStage) -> TeamState {
        TeamState {
            stage,
            phase: TeamPhase::Live,
            staged: Vec::new(),
        }
    }

    pub(crate) fn terminal(&self) -> bool {
        self.phase.terminal()
    }

    pub(crate) fn ready(&self) -> bool {
        matches!(self.stage, TeamStage::Ready { .. }) && !self.terminal()
    }

    pub(crate) fn constructing(&self) -> bool {
        matches!(self.stage, TeamStage::Constructing) && !self.terminal()
    }

    pub(crate) fn default_entry(&self) -> usize {
        match (&self.stage, self.terminal()) {
            (TeamStage::Ready { default_entry }, false) => *default_entry,
            _ => 0,
        }
    }

    pub(crate) fn set_default_entry(&mut self, va: usize) {
        self.stage = TeamStage::Ready { default_entry: va };
    }

    /// 收下：活着就转暂停，已经暂停则无操作；注定要死或已撤离不允许。
    pub(crate) fn debark(&mut self) -> Result<(), UnitFail> {
        match self.phase {
            TeamPhase::Live => self.phase = TeamPhase::Paused,
            TeamPhase::Paused => {}
            TeamPhase::Doomed | TeamPhase::Ousted => return Err(UnitFail::Denied),
        }
        Ok(())
    }

    /// 换回：暂停就恢复活着，活着则无操作；注定要死或已撤离不允许。
    pub(crate) fn embark(&mut self) -> Result<(), UnitFail> {
        match self.phase {
            TeamPhase::Paused => self.phase = TeamPhase::Live,
            TeamPhase::Live => {}
            TeamPhase::Doomed | TeamPhase::Ousted => return Err(UnitFail::Denied),
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn staged_len(&self) -> usize {
        self.staged.len()
    }

    pub(crate) fn push_staging(&mut self, item: Staging) {
        self.staged.push(item);
    }

    pub(crate) fn find_staging(&self, hit: impl FnMut(&Staging) -> bool) -> Option<usize> {
        self.staged.iter().position(hit)
    }

    pub(crate) fn staged_at(&self, index: usize) -> Option<Staging> {
        self.staged.get(index).cloned()
    }

    pub(crate) fn remove_staging(&mut self, index: usize) {
        if index < self.staged.len() {
            self.staged.remove(index);
        }
    }

    /// 摘走整表（连容量）：状态里留一块 0 容量的空壳，丢它不碰分配器。
    pub(crate) fn take_staging(&mut self) -> Vec<Staging> {
        core::mem::take(&mut self.staged)
    }

    /// 整表换进去，旧表交回调用方在锁外丢；锁内不搬元素也不释放内存。
    pub(crate) fn swap_staging(&mut self, items: Vec<Staging>) -> Vec<Staging> {
        core::mem::replace(&mut self.staged, items)
    }
}
