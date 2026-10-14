//! 槽里被锁住的那部分：在座任务、饥饿队列、队列起等的时刻。

use alloc::sync::Arc;
use core::time::Duration;

use crate::core::work::room::chain::Chain;
use crate::core::work::room::deadline;
use crate::core::work::unit::task::Task;
use crate::machine::clock;

use super::READY_MS;

pub(in super::super) struct Inner {
    pub(in super::super) running: Option<Arc<Task>>,
    pub(in super::super) chain: Chain,
    pub(in super::super) ready_since: Option<u64>,
}

impl Inner {
    /// 本核这一段能跑多久：队列空着按盲上限，等着了按起等时刻折算出余量。
    pub(in super::super) fn ready_ceiling(&self) -> u64 {
        self.ready_since.map_or(deadline::blind_ceiling(), |since| {
            let budget = clock::duration_to_ticks(Duration::from_millis(READY_MS));
            budget.saturating_sub(clock::now().as_ticks().saturating_sub(since))
        })
    }

    pub(in super::super) fn starved_is_empty(&self) -> bool {
        self.chain.is_empty()
    }
}
