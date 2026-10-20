//! 本次绑定在停链上走到哪一步。
//!
//! 每一步都是“一帧只推一次”的那一档：要等硬件就停在当前格，引用与在途命令原样保留。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// 引用已入账，物理页坐标尚待取得。
    Acquiring,
    /// 帧引用已取得，翻译还没发布。
    Acquired,
    /// 已记“可能可达”，表与目录正在写、失效与 fence 还没确认。
    Publishing,
    /// 发布部分失败；只允许撤销与结清。
    PublishFailed,
    /// 翻译已发布（设备可能可达）。
    Published,
    /// 已开始撤翻译（目录项已清，fence 还没确认）。
    Invalidating,
    /// 翻译已撤、fence 已确认；帧仍持有。
    Invalidated,
    /// 正在关本轮队列。
    Quiescing,
    /// 队列已关、回读已确认。
    Quiet,
    /// 正在解除引用与本地窗。
    Releasing,
    /// 责任已结清（记录随后销毁，设备可重绑）。
    Released,
}
