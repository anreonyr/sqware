// 地址空间 — MMU 子系统的核心抽象
//
// Space 拥有一个**随运行模式**的根页表与全部自有物理帧，提供虚拟→物理映射、
// 权限管理、地址翻译等高层操作。空间种类由 [`SpaceKind`] 显式区分：内核空间
// （ASID 0，全局唯一）与用户空间（独立 ASID），构造统一走 [`SpaceBuilder`]。
// 布局几何随模式（lower/upper，见 `memory::manager::mode`）。
//
// 文件夹结构（簿记模型三件套 + 窗口 + 主类型）：
//   map       — VA→PA 簿记的原子单元（[`Map`] / [`MapKind`]）
//   dynamic   — 动态侧公共核心：窗口原语（区间分配器 + 子 Map 表，[`Dynamic`]）
//   window    — 按种类的窗口类型（[`StackWindow`] / [`FrameWindow`] / [`HeapWindow`]，
//                各自构造与生命周期操作，见 `window/mod.rs`）
//   durable   — 常数侧：页表树 + 常数映射表（[`Durable`]）
//   core      — 主类型 [`Space`] / [`SpaceBuilder`] / [`Segments`] + 业务流程
//                （含内部组合层 [`SpaceInner`] + 锁约定 + `with`/`with_flush` 事务入口）
//
// 窗口事务统一经 `Space::with` / `Space::with_flush`（锁恰好一次 + 按需刷 TLB）；
// 增加新窗口种类 = `window/` 下新类型 + `SpaceInner` 字段 + `windows()`/
// `windows_mut()` 登记一处——`Space` 的 impl 零改动。
//
// 簿记模型（三层语义，详见各子文件）：
//   Durable — 常数侧（页表树 + 常数映射）
//   Dynamic — 动态侧核心（窗口原语）；窗口种类语义在其上包装（`window/`）
//   Map     — VA→PA 原子单元（不变量：frames[i] ↔ va + i·PAGE_SIZE）

mod core;
mod durable;
mod dynamic;
mod map;
pub(crate) mod window;

pub use core::{Space, SpaceBuilder};
pub use map::MapKind;

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