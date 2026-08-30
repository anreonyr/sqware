// 地址空间 — MMU 子系统的核心抽象
//
// Space 拥有一个**随运行模式**的根页表与全部自有物理帧，提供虚拟→物理映射、
// 权限管理、地址翻译等高层操作。空间种类由 [`SpaceKind`] 显式区分：内核空间
// （ASID 0，全局唯一）与用户空间（独立 ASID），构造统一走 [`SpaceBuilder`]。
// 布局几何随模式（lower/upper，见 `memory::manager::mode`）。
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

pub use core::{Space, SpaceBuilder};
pub(crate) use core::Span;
pub(crate) use map::Pending;
pub(crate) use seg::Seg;

/// 空间种类 — 显式区分内核空间与用户空间。
///
/// 内核空间 ASID 恒 0、全局唯一；用户空间各自持有独立 ASID（1..=65535），
/// 构造时经 [`super::asid::allocate`] 分配、`Drop` 释放。
///
/// 布局几何常量见 `crate::layout`；堆窗口由装载期按 image_end 派生。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    /// 内核空间（ASID 0）。
    Kernel,
    /// 用户空间（独立 ASID）。
    User { asid: usize },
}

impl SpaceKind {
    /// 本空间的 ASID（内核恒 0）。
    pub fn asid(&self) -> usize {
        match self {
            SpaceKind::Kernel => 0,
            SpaceKind::User { asid } => *asid,
        }
    }
}
