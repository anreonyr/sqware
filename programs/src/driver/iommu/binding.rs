//! 本次绑定的全部事实：设备号、执行方、IOVA 窗、那一枚来源窗与它的真实页起址。
//!
//! 帧引用**不在本记录里**：它在 `hold` 那一本账上（`Holds`），一段保留到硬件确认停止
//! 为止；本记录只留它的 IOVA 窗、页数与发布进度。

use alloc::vec::Vec;
use env::{PieToken, TaskId};

use super::hold::DeviceRange;
use super::phase::Phase;
use super::invalidation::Invalidation;

pub struct Binding {
    pub serial: u64,
    /// 持有账上那一段的编号（`Holds::register` 给的）。
    pub hold: usize,
    pub requester: u16,
    pub execution: TaskId,
    /// 本绑定给设备看到的那一段 IOVA 窗（也是持有账那一段的坐标）。
    pub range: DeviceRange,
    pub pages: usize,
    pub access: u32,
    /// 执行方交进来的那一枚窗（本域表里的号）。
    pub source: PieToken,
    /// 那一窗每一页的真实物理起址（发布时写进 Sv39 叶项）。
    pub physical: Vec<u64>,
    pub publish: Invalidation,
    pub retire: Invalidation,
    /// 目录项与页表写过了吗（重试不重写）。
    pub programmed: bool,
    /// 本轮队列抬起来了吗（重试不重抬）。
    pub queued: bool,
    pub phase: Phase,
    /// 实际失败方、操作与原失败码（Hub 那本账据此记原始失败）。
    pub failure: Option<(u8, u8)>,
}
