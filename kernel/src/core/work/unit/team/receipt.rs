use alloc::vec::Vec;

use env::{TaskId, TeamId};

/// 一条收据：某个家长（`owner`）还在哪些子团队根（`roots`）上欠着一次退出。
pub(crate) struct Receipt {
    pub owner: TaskId,
    pub roots: Vec<TeamId>,
}
