// VA 段分配 — 段实体 + 选段枚举（替代 memory::allocator::interval）。
//
// 段 = 实体：拥有几何 `[base, edge)` 与已分配块表（start → len）。分配/释放直接
// 在段上做；无锁（全部在 Space 事务内独占）、无注册（构造即定，见 SpaceInner）。
//
// 中心意象：**返回地址区间**（管理未映射 VA 窗口，不实现 `core::alloc::Allocator`）；
// 记账按**存活块数**（每块一条 BTreeMap 条目）——段再大也零 up-front 成本。
// 段内互不重叠（lowest first-fit 保证）；段间互不重叠由几何隔离（user 段与
// kernel 帧段分处用户/内核半区，见 layout.rs `validate`）。
//
// 区别于 interval.rs：Segment 自持数据（无 Arc、无锁、无 accessor/register）——
// 一个 Space 持有两段（user / kernel），分配全在 `Space::with` 事务内串行。

use alloc::alloc::AllocError;
use alloc::collections::BTreeMap;

/// 段 — 一段虚拟地址区间的几何身份（选段/定位段用）。
///
/// 只作「从哪段取 / 归哪段」的参数，不承载几何或分配表（几何在 [`Segment`]
/// 实体字段）。`User` 段装栈/堆/dock 视图；`Kernel` 段装线程 trap 帧。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seg {
    /// 用户半区 `[free_base, upper)` — 栈/堆/dock 共用。
    User,
    /// 内核 trap 帧常量区 `[TEAM_FRAME_BASE, +SIZE)`，S-only。
    Kernel,
}

/// 段实体 — 几何 + 已分配块表。
///
/// 段内 lowest first-fit：沿分配表扫空隙，取**最低**够大者落块。
/// 无锁：本段的分配/释放在 Space 事务内独占（见 `Space::with` 锁约定）。
#[derive(Debug)]
pub(crate) struct Segment {
    /// 段低界（页对齐）。
    base: usize,
    /// 段高界（开区间）。
    edge: usize,
    /// 已分配块表：起始地址 → 长度（键序互斥，段内空隙 = 相邻键之间）。
    allocated: BTreeMap<usize, usize>,
}

impl Segment {
    /// 构造一段 `[base, edge)`（调用方保证 base 页对齐、edge 可达）。
    pub(crate) fn new(base: usize, edge: usize) -> Self {
        Self {
            base,
            edge,
            allocated: BTreeMap::new(),
        }
    }

    /// 段内 lowest first-fit，返回块基址。
    ///
    /// `size == 0` 按 1 处理（与 interval.rs 行为一致，保持分配语义）。
    ///
    /// # Errors
    ///
    /// 段内无足够连续空隙 → [`AllocError`]。
    pub(crate) fn allocate(&mut self, size: usize) -> Result<usize, AllocError> {
        let size = size.max(1);
        let mut cursor = self.base;
        for (&start, &len) in self.allocated.iter() {
            // 空隙 [cursor, start)：候选
            if start.saturating_sub(cursor) >= size {
                self.allocated.insert(cursor, size);
                return Ok(cursor);
            }
            cursor = cursor.max(start.saturating_add(len));
        }
        // 段尾空隙 [cursor, edge)
        if self.edge.saturating_sub(cursor) >= size {
            self.allocated.insert(cursor, size);
            return Ok(cursor);
        }
        Err(AllocError)
    }

    /// 精确匹配释放 `(addr, size)`：条目存在且长度相等 → 删除 → `true`。
    ///
    /// 未分配 / 长度不匹配 / 越界 → `false`。释放失败须由调用方显式处置
    /// （`Space::release` 中 panic，避免静默段泄漏）。
    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> bool {
        match self.allocated.get(&addr) {
            Some(&len) if len == size => {
                self.allocated.remove(&addr);
                true
            }
            _ => false,
        }
    }
}
