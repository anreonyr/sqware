//! 取得一次私有 IOVA 映射：完整 Domain 身份 ＋ 完整 Hub 绑定号 ＋ 执行方与来源窗。

use env::{PieToken, TaskId};

/// 61 字节；报文不带 caller、不带 IOVA（私有分配在核心内一次完成）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 61)]
pub struct Acquire {
    pub op: u8,
    pub engine: TaskId,
    pub key: u64,
    pub hub: TaskId,
    pub serial: u64,
    pub execution: TaskId,
    pub source: PieToken,
    pub access: u32,
    pub back: PieToken,
}
