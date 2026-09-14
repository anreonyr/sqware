// VA 段分配 — 段实体 + 选段枚举（替代 memory::allocator::interval）。
//
// 段 = 实体：拥有几何 `[base, edge)` 与已分配块表（start → len）。分配/释放直接
// 在段上做；无锁（全部在 Space 事务内独占）、无注册（构造即定，见 SpaceInner）。
//
// 中心意象：**返回地址区间**（管理未映射 VA 窗口，不实现 `core::alloc::Allocator`）；
// 记账按**存活块数**（每块一条表项）——段再大也零 up-front 成本。
// 段内互不重叠（lowest first-fit 保证）；段间互不重叠由几何隔离（user 段与
// kernel 帧段分处用户/内核半区，见 layout.rs `validate`）。
//
// 区别于 interval.rs：Segment 自持数据（无 Arc、无锁、无 accessor/register）——
// 一个 Space 持有两段（user / kernel），分配全在 `Space::with` 事务内串行。
//
// # 块表用有序 `Vec` 而不是 `BTreeMap`：为了"增长可失败"
//
// 取段这一步只发生在**能失败**的装配入口里（`SpaceInner::allocate`，上层把失败翻成
// `-4`/`-1`），所以它该答 `OutOfMemory`，而不是走 `BTreeMap` 那条没有 `try_reserve`
// 的不可失败叶节点分配（同类实测：368 B 那次）。
// 复杂度不变：lowest first-fit 今天也是沿表顺序扫一遍。

use alloc::alloc::AllocError;
use alloc::vec::Vec;

/// 段 — 一段虚拟地址区间的几何身份（选段/定位段用）。
///
/// 只作「从哪段取 / 归哪段」的参数，不承载几何或分配表（几何在 [`Segment`]
/// 实体字段）。`Normal` 段装栈/堆/共享页视图（Pole）；`Kernel` 段装线程 trap 帧。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    /// Normal 半区 `[free_base, upper)` — 栈/堆/共享页共用。
    Normal,
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
    /// 已分配块表：`(起址, 长度)`，**按起址升序**（段内空隙 = 相邻两条之间）。
    allocated: Vec<(usize, usize)>,
}

impl Segment {
    /// 构造一段 `[base, edge)`（调用方保证 base 页对齐、edge 可达）。**不分配**。
    pub(crate) fn new(base: usize, edge: usize) -> Self {
        Self {
            base,
            edge,
            allocated: Vec::new(),
        }
    }

    /// 段内 lowest first-fit，返回块基址。
    ///
    /// `size == 0` 按 1 处理（与 interval.rs 行为一致，保持分配语义）。
    ///
    /// # Errors
    ///
    /// 段内无足够连续空隙 → [`AllocError`]；块表扩不出来（内存耗尽）→ [`AllocError`]。
    pub(crate) fn allocate(&mut self, size: usize) -> Result<usize, AllocError> {
        let size = size.max(1);
        let mut cursor = self.base;
        let mut at = self.allocated.len();
        for (i, &(start, len)) in self.allocated.iter().enumerate() {
            // 空隙 [cursor, start)：候选
            if start.saturating_sub(cursor) >= size {
                at = i;
                break;
            }
            cursor = cursor.max(start.saturating_add(len));
        }
        if at == self.allocated.len() {
            // 段尾空隙 [cursor, edge)
            if self.edge.saturating_sub(cursor) < size {
                return Err(AllocError);
            }
        }
        // **唯一会分配的一步**（紧贴 insert）：失败即当场答错，段表一字未改。
        self.allocated.try_reserve(1).map_err(|_| AllocError)?;
        self.allocated.insert(at, (cursor, size));
        Ok(cursor)
    }

    /// 精确匹配释放 `(addr, size)`：条目存在且长度相等 → 删除 → `true`。
    ///
    /// 未分配 / 长度不匹配 / 越界 → `false`。释放失败须由调用方显式处置
    /// （`Space::release` 中 panic，避免静默段泄漏）。**不分配**。
    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> bool {
        match self.allocated.iter().position(|&(start, _)| start == addr) {
            Some(at) if self.allocated[at].1 == size => {
                self.allocated.remove(at);
                true
            }
            _ => false,
        }
    }

    /// 只读校验：`(addr, size)` 当前是否为本段的一个已分配块。
    ///
    /// 存在理由：把拆除路径的失败域**前移到事务内**，使真正的 [`Self::deallocate`]
    /// 能延后到跨核清退之后（还段 = VA 可被复用，远核旧条目会污染新映射）。
    pub(crate) fn holds(&self, addr: usize, size: usize) -> bool {
        self.allocated
            .iter()
            .any(|&(start, len)| start == addr && len == size)
    }
}
