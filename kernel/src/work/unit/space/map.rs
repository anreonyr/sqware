// Map — VA→PA 簿记的原子单元。
//
// 语义：本映射覆盖 VA 区间 `[va, va + size)`；`frames[i]` 是 VA
// `va + i·PAGE_SIZE` 处**已物化**页的物理帧持有者（**不变量**：帧 i ↔ va + i·PAGE_SIZE，
// 未物化页不在数组里——懒映射按触序前缀登记，guard 页永不入帧）。
// `pending` 表达**未物化页的行为**（与帧所有权正交）：
//   None       — 无未物化页（全物化）。拥有映射满帧；借用映射（DRAM 恒等、
//                 trampoline、dock 视图）空帧——leaf 在册、物理帧归外部。
//   Some(Lazy) — 未物化页缺页时分配零页（懒映射：mmap/declare/栈体）。
//   Some(Guard)— 未物化且禁止物化：触碰即「预留映射访问」（栈守护页）。
//
// 帧随 Map drop 归还 frame 池（Owned）或 Arc 计数归零归还（Borrowed）——
// 所有权即回收，无遍历页表树、无手写 deallocate。

use core::num::NonZeroUsize;

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::{Frame, FrameState};

/// 未物化页的行为（Map 级）— 与帧所有权正交。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    /// 缺页时物化零页（懒分配，帧按触序注入 `frames` 前缀）。
    Lazy,
    /// 禁止物化：触碰即「预留映射访问」（栈守护页）。
    Guard,
}

/// 虚拟→物理映射 — 簿记的原子单元。
///
/// 语义见文件头。`frames` 只含已物化页（按序前缀）；`pending` 描述未物化页行为。
#[derive(Debug)]
pub(crate) struct Map {
    pub(super) va: VirtAddr,
    pub(super) size: NonZeroUsize,
    pub(super) flags: PteFlags,
    /// 未物化页行为（None = 全物化：拥有满帧 / 借用空帧）。
    pub(super) pending: Option<Pending>,
    /// 已物化页帧（按序前缀；未物化页不在数组）。
    pub(super) frames: Vec<FrameState>,
}

impl Map {
    /// 构造（size 必须非零——调用方保证，见各入口的校验）。
    pub(super) fn new(
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
        frames: Vec<FrameState>,
    ) -> Self {
        Self {
            va,
            size: NonZeroUsize::new(size).expect("map size must be non-zero"),
            flags,
            pending,
            frames,
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    pub(super) fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 第 `idx` 页是否已物化（已在页表）。
    ///
    /// - `pending: None` → 全物化（拥有映射满帧 / 借用映射 leaf 在册）→ true。
    /// - `Some(Lazy)` → 已触页（`frames.len() > idx`）→ true；未触 → false。
    /// - `Some(Guard)` → 永不物化 → false。
    ///
    /// 缺页 / mprotect 判「该页已物化」用；借用映射恒 true（leaf 在册）。
    pub(super) fn is_materialized(&self, idx: usize) -> bool {
        match self.pending {
            None => true,
            Some(Pending::Lazy) => idx < self.frames.len(),
            Some(Pending::Guard) => false,
        }
    }

    /// 注入一帧（保持不变量；调用方保证序号连续且不越界）。
    pub(super) fn inject(&mut self, frame: Frame) {
        debug_assert!(
            self.frames.len() < self.size.get() / PAGE_SIZE,
            "map {:#x} frame overflow",
            self.va.as_usize()
        );
        self.frames.push(FrameState::Owned(frame));
    }
}
