// Dynamic — 动态窗口核心 + 区间分配器 + 子 Map 表（簿记模型第二层：动态侧）。
//
// 各持一个 [`IntervalAllocator`]（区间树，∝ 存活块）细分窗口内 VA；每次分配的
// 一块区间即一个子 Map（`children`）——heap 块 / 栈 slot / 线程帧。
// `children` 记录本窗口已分配出去的每块区间，与区间分配器的存活块一一对应——
// 释放即从 children 移除（帧随 Map drop 归还），窗口可安全复用（PTE 已清、
// 无残留映射）。窗口的身份在种类类型上（`window` 子模块）。
//
// 构造与生命周期操作按种类随窗口类型走（`window/stack.rs`、`window/frame.rs`、
// `window/heap.rs`）；本文件只留 kind 无关的窗口核心原语。

use core::num::NonZeroUsize;

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::interval::{Direction, IntervalAllocator};
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::map::{Map, MapKind};

/// 动态窗口 — 固定 VA 区间 + 区间分配器 + 结构性绑定的子映射表。
///
/// `children` 记录本窗口已分配出去的每块区间（heap 块 / 栈 slot / 线程帧），
/// 与区间分配器的存活块一一对应——释放即从 children 移除（帧随 Map drop 归还），
/// 窗口可安全复用（PTE 已清、无残留映射）。窗口身份在种类类型上（`window`
/// 子模块的 `StackWindow` / `FrameWindow` / `HeapWindow`）。
#[derive(Debug)]
pub struct Dynamic {
    pub(super) va: VirtAddr,
    pub(super) size: NonZeroUsize,
    pub(super) allocator: IntervalAllocator,
    pub(super) children: Vec<Map>,
}

impl Dynamic {
    /// 构造窗口 `[base, edge)`（区间分配器，内存 ∝ 存活块，无 eager）。
    pub(super) fn window(base: usize, edge: usize) -> Self {
        Self {
            va: VirtAddr::from_raw(base),
            size: NonZeroUsize::new(edge - base).expect("window size non-zero"),
            allocator: IntervalAllocator::new(base, edge),
            children: Vec::new(),
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    pub(super) fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 从区间树分配一块 VA，登记一个空帧子 Map（懒分配：帧由调用方随后注入）。
    ///
    /// 返回分配基址（Copy，可锁外使用）；`size` 由调用方自知。`owner` = 所属
    /// 线程 id（私有资源如线程帧 / 栈 slot 用于退出回收定位；共享资源如堆块
    /// 传 `None`）。
    pub(super) fn allocate(
        &mut self,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
        owner: Option<usize>,
    ) -> Result<VirtAddr, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let (base, size) = self
            .allocator
            .allocate(size, Direction::Rise)
            .map_err(|_| MapError::OutOfMemory)?;
        let va = VirtAddr::from_raw(base);
        self.children
            .push(Map::new(va, size, flags, kind, Vec::new(), owner));
        Ok(va)
    }

    /// 释放一块 VA：区间精确匹配成功后移除重叠子 Map（帧随 drop 归还）。
    pub(super) fn deallocate(&mut self, va: VirtAddr, size: usize) -> bool {
        if self.allocator.deallocate(va.as_usize(), size).is_err() {
            return false;
        }
        let end = va.as_usize().saturating_add(size);
        self.children.retain(|m| {
            !(va.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize())
        });
        true
    }

    /// 断绝与 `owner` 的全部子 Map 关系（帧随 drop 归还），返回被覆盖区间
    /// `[min_va, min_va + len)`（供调用方清 PTE 后按整区间归还）。
    ///
    /// 线程退役回收用：栈 slot 的守护页/栈体两子 Map 共享同一 owner，一并摘除。
    pub(super) fn disown(&mut self, owner: usize) -> Option<(VirtAddr, usize)> {
        let mut min = usize::MAX;
        let mut max = 0usize;
        let before = self.children.len();
        self.children.retain(|m| {
            if m.owner == Some(owner) {
                min = min.min(m.va.as_usize());
                max = max.max(m.va.as_usize().saturating_add(m.size.get()));
                false
            } else {
                true
            }
        });
        (self.children.len() != before).then(|| (VirtAddr::from_raw(min), max - min))
    }
}
