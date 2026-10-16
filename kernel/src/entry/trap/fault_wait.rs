use alloc::sync::Arc;

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::{FaultKind, FaultOutcome, Nest};

/// 一次页错误的等待上下文：纯探针重查所需的地址空间、地址、访问种类与
/// **本次登记的那段关系身份**。
///
/// 它是条件等待谓词的整个捕获，谓词只在 work 的提交边界内跑一次：这里不推进
/// 失效、不物化，只有 `Nest::probe_fault` 这一个只读口。
pub(crate) struct FaultWait {
    nest: Arc<Nest>,
    address: VirtAddr,
    kind: FaultKind,
    expected: usize,
}

impl FaultWait {
    pub(crate) fn new(
        nest: Arc<Nest>,
        address: VirtAddr,
        kind: FaultKind,
        expected: usize,
    ) -> Self {
        Self {
            nest,
            address,
            kind,
            expected,
        }
    }

    /// 只有“这一页仍然只能等，而且等的正是本次这段关系”才算还没就绪。
    ///
    /// 身份变了就必须立刻回去重探：旧身份已经被清过场、也 signal 过了，
    /// 再睡在旧键上就永远等不到人；新身份的可物化、已经可用或已消失都是重探
    /// 就能决定的。
    pub(crate) fn ready(&self) -> bool {
        !matches!(
            self.nest.probe_fault(self.address, self.kind),
            FaultOutcome::Wait(view) if view == self.expected
        )
    }
}
