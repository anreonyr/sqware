// Map — VA→PA 簿记的原子单元。
//
// 语义：本映射覆盖 VA 区间 `[va, va + size)`；`frames` 是**页序 → 帧**的
// 稀疏表（键 i ↔ VA `va + i·PAGE_SIZE`；未物化页不在表——懒映射按触页登记、
// guard 页永不入帧）。`pending` 表达**未物化页的行为**（与帧所有权正交）：
//   None       — 无未物化页（全物化）。拥有映射满帧；借用映射（DRAM 恒等、
//                 trampoline、dock 视图）空帧——leaf 在册、物理帧归外部。
//   Some(Lazy) — 未物化页缺页时分配零页（懒映射：mmap/declare/栈体）。
//   Some(Guard)— 未物化且禁止物化：触碰即「预留映射访问」（栈守护页）。
//
// 帧随 Map drop 归还 frame 池（Owned）或 Arc 计数归零归还（Shared）——
// 所有权即回收，无遍历页表树、无手写 deallocate。
//
// 部分拆除（`remove`）按洞分裂：`carve` 摘洞内帧并用 `split_off` 重排右段键
// ——洞是**结构上的**（键区间缺省），不是额外 holes 字段；稀疏表让
// `is_materialized` / `clear` 只随已物化键走（O(触页数)，非 O(段大小)）。

use alloc::collections::BTreeMap;
use core::num::NonZeroUsize;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::{Frame, FrameState};

use super::core::Salvage;

/// 未物化页的行为（Map 级）— 与帧所有权正交。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    /// 缺页时物化零页（懒分配，帧按触页登记）。
    Lazy,
    /// 禁止物化：触碰即「预留映射访问」（栈守护页）。
    Guard,
}

/// 缺页分派：`va` 所在页的映射/物化态（`pending_state` 返回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingState {
    /// 无映射。
    Absent,
    /// 已物化 / 借用（pending None）——不该缺页。
    Materialized,
    /// 懒：缺页物化零页。
    Lazy,
    /// 守卫：触碰即「预留映射访问」。
    Guard,
}

/// 虚拟→物理映射 — 簿记的原子单元。
///
/// 语义见文件头。`frames` 稀疏（只含已物化页）；`pending` 描述未物化页行为。
#[derive(Debug)]
pub(crate) struct Map {
    pub(super) va: VirtAddr,
    pub(super) size: NonZeroUsize,
    pub(super) flags: PteFlags,
    /// 未物化页行为（None = 全物化：拥有满帧 / 借用空帧）。
    pub(super) pending: Option<Pending>,
    /// 已物化页帧（页序键；未物化页不在表）。
    pub(super) frames: BTreeMap<usize, FrameState>,
}

impl Map {
    /// 构造（size 必须非零——调用方保证，见各入口的校验）。
    pub(super) fn new(
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
        frames: BTreeMap<usize, FrameState>,
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

    /// 第 `idx` 页是否已物化（已在页表）——**点查询**。
    ///
    /// - `pending: None` → 全物化（拥有映射满帧 / 借用映射 leaf 在册）→ true。
    /// - `Some(Lazy)` → 已触页（`frames` 含键）→ true；未触 → false。
    /// - `Some(Guard)` → 永不物化 → false。
    ///
    /// 运行路径（拆除 / 改权）不用点查询逐页问，用 [`Self::runs`] 的段枚举。
    #[cfg(feature = "audit")]
    pub(super) fn is_materialized(&self, idx: usize) -> bool {
        match self.pending {
            None => true,
            Some(Pending::Lazy) => self.frames.contains_key(&idx),
            Some(Pending::Guard) => false,
        }
    }

    /// 把 `[lo_pg, hi_pg)` 内**已物化**的页折叠成连续段，逐段交 `apply`——
    /// [`Self::is_materialized`] 的**区间版**，同一判据的唯一另一种形态。
    ///
    /// 借用/全物化 = 整段一次；懒区 = `frames` 的键（BTreeMap 有序，天然可折叠）；
    /// guard = 空集。折叠即复杂度：调用次数随**触页段数**走、不随区间大小走
    /// （1 TiB 懒区触 4 页 = 1 段），拆除/改权因此与段大小解耦。
    pub(super) fn runs(&self, lo_pg: usize, hi_pg: usize, mut apply: impl FnMut(VirtAddr, usize)) {
        match self.pending {
            None => apply(self.va + lo_pg * PAGE_SIZE, (hi_pg - lo_pg) * PAGE_SIZE),
            Some(Pending::Lazy) => {
                let mut run: Option<(usize, usize)> = None;
                for (&pg, _) in self.frames.range(lo_pg..hi_pg) {
                    run = match run {
                        Some((start, len)) if start + len == pg => Some((start, len + 1)),
                        Some((start, len)) => {
                            apply(self.va + start * PAGE_SIZE, len * PAGE_SIZE);
                            Some((pg, 1))
                        }
                        None => Some((pg, 1)),
                    };
                }
                if let Some((start, len)) = run {
                    apply(self.va + start * PAGE_SIZE, len * PAGE_SIZE);
                }
            }
            Some(Pending::Guard) => {}
        }
    }

    /// 注入一帧（保持不变量：帧键 ↔ va + 键·PAGE；调用方保证键不越界、不重复）。
    pub(super) fn inject(&mut self, idx: usize, frame: Frame) {
        debug_assert!(
            idx < self.size.get() / PAGE_SIZE,
            "map {:#x} frame overflow",
            self.va.as_usize()
        );
        let old = self.frames.insert(idx, FrameState::Owned(frame));
        debug_assert!(
            old.is_none(),
            "map {:#x} double inject @page {idx}",
            self.va.as_usize()
        );
    }

    /// 把洞 `[lo_pg, hi_pg)`（页序，半开）挖掉——统一拆除中**部分覆盖**的 Map。
    ///
    /// 洞内帧**交料箱**（`salvage`）：清退到齐后才归还，不得在此 drop（远核可能
    /// 仍持旧条目）。本 Map 收缩为洞左段（`lo_pg == 0` 时本 Map 重绕成洞右段，
    /// 帧键重排 −hi_pg）；洞右段存在时作为新 Map 返回（键已重排）。
    ///
    /// 前置：`lo_pg < hi_pg ≤ pages` 且**非全覆盖**（全覆盖由调用方直接整张
    /// 摘除，不至此）——返回后本 Map 恒存活，右段与否看返回值。
    pub(super) fn carve(
        &mut self,
        lo_pg: usize,
        hi_pg: usize,
        salvage: &mut Salvage,
    ) -> Option<Map> {
        let pages = self.size.get() / PAGE_SIZE;
        debug_assert!(lo_pg < hi_pg && hi_pg <= pages);
        let tail = self.frames.split_off(&hi_pg); // 键 ≥ hi_pg → 右段
        let hole = self.frames.split_off(&lo_pg); // [lo_pg, hi_pg) → 洞
        // 洞内帧交料箱（键重排 −lo_pg，与洞基址对齐）
        salvage.take_map(Map::new(
            self.va + lo_pg * PAGE_SIZE,
            (hi_pg - lo_pg) * PAGE_SIZE,
            self.flags,
            self.pending,
            hole.into_iter().map(|(k, v)| (k - lo_pg, v)).collect(),
        ));
        // 右段帧键重排：k − hi_pg
        let right_frames: BTreeMap<usize, FrameState> =
            tail.into_iter().map(|(k, v)| (k - hi_pg, v)).collect();
        if lo_pg == 0 {
            // 洞在头：本 Map 重绕成右段（调用方分支保证 hi_pg < pages）
            self.va += hi_pg * PAGE_SIZE;
            self.size = NonZeroUsize::new((pages - hi_pg) * PAGE_SIZE).expect("non-zero");
            self.frames = right_frames;
            return None;
        }
        self.size = NonZeroUsize::new(lo_pg * PAGE_SIZE).expect("non-zero");
        if hi_pg == pages {
            return None; // 洞在尾：无右段
        }
        Some(Map {
            va: self.va + hi_pg * PAGE_SIZE,
            size: NonZeroUsize::new((pages - hi_pg) * PAGE_SIZE).expect("non-zero"),
            flags: self.flags,
            pending: self.pending,
            frames: right_frames,
        })
    }
}
