// Dynamic — 窗口子 Map 簿记核心（簿记模型第二层：动态侧），无独立 VA 域。
//
// 段表 + 访问器（`interval.rs`）之后，窗口不再持「分配器 + 段路由」——`Dynamic`
// 只做两件事：① 持本段访问器（`IntervalAllocator`，段信息已内涵）；② 本窗口的
// 子 Map 表（`children`）。每次分配一块 VA（经访问器段内 lowest first-fit）即
// 一个子 Map——heap 块 / 栈 slot / 线程帧。`children` 记录本窗口已分配出去的
// 每块区间，释放即从 children 移除（帧随 Map drop 归还），窗口可安全复用
// （PTE 已清、无残留映射）。窗口的身份仍在种类类型上（`window` 子模块）。
//
// 构造与生命周期操作按种类随窗口类型走（`window/stack.rs`、`window/frame.rs`、
// `window/heap.rs`）；本文件只留 kind 无关的窗口核心原语。

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::interval::IntervalAllocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::map::{Map, MapKind};

/// 动态侧簿记 — 段访问器 + 本窗口子 Map 表。
///
/// 窗口无独立 VA 域：VA 全经访问器出（绑定段 = 构造时选定）；`children` 仍是
/// 本窗口已分配块记录（audit/resolve/unmap 的遍历点）。
#[derive(Debug)]
pub struct Dynamic {
    /// 本窗口段访问器（段信息内涵：构造时绑定的段）。
    pub(super) alloc: IntervalAllocator,
    pub(super) children: Vec<Map>,
}

impl Dynamic {
    /// 构造：绑定段访问器（窗口种类决定绑定何段，见 window 子模块）。
    pub(super) fn new(alloc: IntervalAllocator) -> Self {
        Self {
            alloc,
            children: Vec::new(),
        }
    }

    /// 从访问器分配一块 VA，登记一个空帧子 Map（懒分配：帧由调用方随后注入）。
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
        let base = self
            .alloc
            .allocate(size)
            .map_err(|_| MapError::OutOfMemory)?;
        let va = VirtAddr::from_raw(base);
        self.children
            .push(Map::new(va, size, flags, kind, Vec::new(), owner));
        Ok(va)
    }

    /// 释放一块 VA：区间精确匹配成功后移除重叠子 Map（帧随 drop 归还）+ 归还段。
    pub(super) fn deallocate(&mut self, va: VirtAddr, size: usize) -> bool {
        if !self.alloc.deallocate(va.as_usize(), size) {
            return false;
        }
        let end = va.as_usize().saturating_add(size);
        self.children.retain(|m| {
            !(va.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize())
        });
        true
    }

    /// 断绝与 `owner` 的全部子 Map 关系（帧随 drop 归还）并归还段，返回被覆盖
    /// 区间 `[min_va, min_va + len)`（供调用方清 PTE）。
    ///
    /// 线程退役回收用：栈 slot 的守护页/栈体两子 Map 共享同一 owner，一并摘除。
    /// 段归还尽力而为（分配器按精确键删除；覆盖区间与键一致的场景必成功）；
    /// 命中（摘除非空）恒返回区间——PTE 清理由调用方按区间完成，与归还成败无关。
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
        if self.children.len() == before {
            return None;
        }
        let va = VirtAddr::from_raw(min);
        let len = max - min;
        let _ = self.alloc.deallocate(va.as_usize(), len); // 尽力归还（无键则留待 audit 暴露）
        Some((va, len))
    }
}
