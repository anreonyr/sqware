//! 一次绑定的全部事实：Hub 的完整绑定号、执行方、私有 IOVA 段与它自己的引用账。
//!
//! 帧引用不在本记录里：它在本 Domain 的 `Holds` 上，一段保留到硬件确认停止为止。
//! 本记录只留私有分配、发布进度与那一份账上的登记号。

use alloc::vec::Vec;
use env::{PieToken, TaskId};
use hub_api::BindingId;

use super::domain_range::DomainRange;
use super::invalidation::Invalidation;
use super::phase::Phase;

pub struct Binding {
    pub binding: BindingId,
    /// 持有账上那一段的登记号；取得引用之前为空。
    pub hold: Option<usize>,
    pub requester: u16,
    pub execution: TaskId,
    /// 本 Domain 私有分配的 IOVA 段（也是持有账那一段的坐标）。
    pub range: DomainRange,
    pub pages: usize,
    pub access: u32,
    /// 执行方交进来的那一枚窗（本域表里的号）。
    pub source: PieToken,
    /// 那一窗每一页的真实物理起址（发布时写进 Sv39 叶项）。
    pub physical: Vec<u64>,
    pub publish: Invalidation,
    pub retire: Invalidation,
    /// 本 Domain 的目录项与页表写过了吗（重试不重写）。
    pub programmed: bool,
    pub phase: Phase,
    /// 实际失败方、操作与原失败码（Hub 那本账据此记原始失败）。
    pub failure: Option<(u8, u8)>,
}
