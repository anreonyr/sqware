// 地址空间 — MMU 子系统的核心抽象
//
// Space 拥有一个**随运行模式**的根页表与全部自有物理帧，提供虚拟→物理映射、
// 权限管理、地址翻译等高层操作。空间种类由 [`SpaceKind`] 显式区分（S 态页表 /
// U 态页表），空间身份由独立字段 `Asid` 承载（0 = 内核空间），构造统一走
// [`SpaceBuilder`]。布局几何随模式（lower/upper，见 `memory::manager::mode`）。
//
// 文件夹结构（纯映射簿记 + 段实体 + 窗口适配层）：
//   seg       — 段实体（[`Segment`]，几何 + 已分配块表）+ 选段枚举（[`Seg`]）
//   map       — VA→PA 簿记的原子单元（[`Map`] / [`Pending`]）
//   core      — 主类型 [`Space`] / [`SpaceBuilder`] / [`SpaceInner`] + 映射原语
//   window    — 窗口适配层（[`StackWindow`] / [`FrameWindow`] / [`HeapWindow`] /
//                 [`ShareWindow`]，操作 `Space` 的领域策略，产物统一 [`Span`]）
//
// 簿记模型（三层语义）：
//   Segment — 一段 VA（user 半区 / kernel 帧区），lowest first-fit 出块
//   Span    — 分配/映射的产物（段 + VA + size + 物化帧 PA），回收的输入
//   Map     — VA→PA 原子单元（区间 + 访问属性 + 物化态 + 帧所有权）
//   SpaceInner 持 root 页表树 + 两段 + 唯一 maps 表；窗口方法操作它。

mod core;
mod map;
mod seg;
pub(crate) mod window;

pub(crate) use core::Span;
pub use core::{Space, SpaceBuilder};
pub(crate) use map::{Pending, PendingState};
pub(crate) use seg::Seg;

/// 空间种类 — 页表被哪个特权级使用（单一事实源）。
///
/// `Supervisor` = S 态运行所用的页表（内核空间与 supervisor 域空间同属此类）；
/// `User` = U 态运行所用的页表。**不表达 ASID**——ASID 是空间身份，独立字段
/// （[`Asid`](crate::memory::manager::asid::Asid)）；「是不是内核空间」由
/// `Asid::is_kernel()` 判定，不是本枚举的职责。
///
/// 布局几何常量见 `crate::layout`；堆窗口由装载期按 image_end 派生。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    /// S 态页表（内核空间 / supervisor 域空间）。
    Supervisor,
    /// U 态页表。
    User,
}

impl SpaceKind {
    /// 是否 S 态页表（SPP / tp / 窗口 U 位的单一判据）。
    pub fn is_supervisor(self) -> bool {
        matches!(self, SpaceKind::Supervisor)
    }
}
