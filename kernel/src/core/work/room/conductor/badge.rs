//! 本核在座身份位：一个字。要么是活 `TaskIdent` 的裸指针（连同那份强引用），
//! 要么是末次身份的打包号 —— 最低位打标，与裸指针不相混，0 留给空。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::core::work::room::identity::Identity;
use crate::core::work::unit::task::TaskIdent;

pub(in super::super) struct Badge {
    raw: AtomicUsize,
}

impl Badge {
    const TAG: usize = 1;
    const TEAM_BITS: u32 = 31;
    const TASK_SHIFT: u32 = 1 + Self::TEAM_BITS;
    const TEAM_MASK: usize = (1 << Self::TEAM_BITS) - 1;

    pub(in super::super) const fn new() -> Badge {
        Badge {
            raw: AtomicUsize::new(0),
        }
    }

    const fn tagged(raw: usize) -> bool {
        raw & Self::TAG != 0
    }

    const fn pack(task: TaskId, team: TeamId) -> usize {
        Self::TAG | (team.get() << 1) | (task.get() << Self::TASK_SHIFT)
    }

    const fn unpack(raw: usize) -> (TaskId, TeamId) {
        (
            TaskId::new(raw >> Self::TASK_SHIFT),
            TeamId::new((raw >> 1) & Self::TEAM_MASK),
        )
    }

    fn reclaim(raw: usize) {
        if raw != 0 && !Self::tagged(raw) {
            unsafe { drop(Arc::from_raw(raw as *const TaskIdent)) };
        }
    }

    pub(in super::super) fn seat(&self, ident: &Arc<TaskIdent>) {
        let prev = self
            .raw
            .swap(Arc::into_raw(ident.clone()) as usize, Ordering::AcqRel);
        Self::reclaim(prev);
    }

    pub(in super::super) fn shed(&self, ident: &TaskIdent) {
        let word = Self::pack(ident.id, ident.team.id);
        debug_assert_eq!(
            Self::unpack(word),
            (ident.id, ident.team.id),
            "末次编码往返"
        );
        let prev = self.raw.swap(word, Ordering::AcqRel);
        debug_assert!(prev != 0 && !Self::tagged(prev), "shed 旧载荷非活任务");
        Self::reclaim(prev);
    }

    pub(in super::super) fn read(&self) -> Option<Identity> {
        let raw = self.raw.load(Ordering::Acquire);
        if raw == 0 {
            None
        } else if Self::tagged(raw) {
            let (task, team) = Self::unpack(raw);
            Some(Identity::Last { task, team })
        } else {
            let live = raw as *const TaskIdent;
            unsafe {
                Arc::increment_strong_count(live);
                Some(Identity::Live(Arc::from_raw(live)))
            }
        }
    }
}
