//! 一次移交的结果载荷：迁移结果与它动过的活关系，两条通道交给上层。
//!
//! 清单只表达 core 关系事实——两端各自压着这一页的那些关系——不认识 Room、
//! Task 或 Work；怎么挤醒等待者由上层决定。`must_use` 只提醒把这份清单消费掉，
//! 不强制通知顺序，也不阻止调用方只取 `result` 而丢弃 `affected`。

use alloc::sync::Weak;
use alloc::vec::Vec;

use crate::core::memory::nest::View;

use super::TransferError;

/// 一次移交的完整结果。
#[must_use = "the affected relations must reach whoever wakes their waiters"]
pub struct TransferOutcome {
    /// 本次移交的成败。占槽前的早错也为 `Err`，那时 `affected` 为空。
    pub result: Result<(), TransferError>,
    /// 本次移交动过的活关系：压着源页或目标页的那些。占槽前失败时为空。
    pub affected: Vec<Weak<View>>,
}
