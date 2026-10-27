//! 一步（发布／撤销／关队列／解除）：完整 Domain 身份 ＋ 完整 Hub 绑定号 ＋ 理由。

use env::{PieToken, TaskId};

/// 42 字节；只有 `release` 使用 `reason`。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 42)]
pub struct Step {
    pub op: u8,
    pub engine: TaskId,
    pub key: u64,
    pub hub: TaskId,
    pub serial: u64,
    pub reason: u8,
    pub back: PieToken,
}
