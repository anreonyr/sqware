//! 身份词汇：本核在座的是谁。
//!
//! 在座者可能是一枚活任务（`Live`，连同它的 `TaskIdent`），也可能是刚从运行槽上
//! 下来的那一位（`Last`，只剩 task/team 两个号）。trap 路径与报告路径都只认这套
//! 词汇；槽里那一个字怎么编，是 `conductor::badge` 自己的事。

use alloc::sync::Arc;

use env::{TaskId, TeamId};

use crate::core::memory::address::PhysAddr;
use crate::core::work::unit::task::TaskIdent;

use super::conductor;
use super::scheduler;

pub enum Identity {
    Live(Arc<TaskIdent>),
    Last { task: TaskId, team: TeamId },
}

impl Identity {
    pub fn task_id(&self) -> usize {
        match self {
            Identity::Live(t) => t.id.get(),
            Identity::Last { task, .. } => task.get(),
        }
    }

    pub fn team_id(&self) -> usize {
        match self {
            Identity::Live(t) => t.team.id.get(),
            Identity::Last { team, .. } => team.get(),
        }
    }

    pub fn trap(&self) -> Option<PhysAddr> {
        match self {
            Identity::Live(t) => Some(t.frame_pa),
            Identity::Last { .. } => None,
        }
    }

    pub fn live(&self) -> Option<&TaskIdent> {
        match self {
            Identity::Live(t) => Some(t),
            Identity::Last { .. } => None,
        }
    }
}

/// 本核在座者的身份；本核还没有运行槽时没有身份可报。
pub fn ident() -> Option<Identity> {
    if !scheduler::installed() {
        return None;
    }
    conductor::current().badge.read()
}
