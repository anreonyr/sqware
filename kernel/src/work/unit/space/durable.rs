// Durable — 常数侧：页表树 + 常数映射表（簿记模型第二层：常数侧）。
//
// `root` 是页表树：硬件页（根/中间表，恰好一帧）+ 子树所有权（帧外 TableNode）——
// 硬件页恰好 4096 B 装不下元数据，子树节点放在帧外的 TableNode 上（见 table.rs）；
// unmap 回收变空的中间表。
//
// `maps` 覆盖空间建立期就确定、生命周期与空间一致的映射：
//   - 内核 DRAM 恒等 / 内核高半区
//   - trampoline 叶 PTE（用户空间只映射不拥有，帧归内核）
//   - 文本段、trap-context 帧
//
// 用户空间不复制 / 不共享内核映射——trampoline 叶 PTE 只映射不拥有（[`Map::frames`]
// 为空）；其余帧（文本、trap-context、堆块、栈体）全归本空间所有。Drop 递归归还
// 全部页表帧。

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::mode;
use crate::memory::manager::table::{Frame, MapError, TableNode};

use super::map::Map;

/// 常数侧 — 页表树（TableNode）+ 常数映射表。
///
/// `root` 是页表树：硬件页（根/中间表，恰好一帧）+ 子树所有权（帧外 TableNode）；
/// unmap 回收变空的中间表。`maps` 覆盖空间建立期就确定、生命周期与空间一致的
/// 映射（内核 DRAM 恒等/高半区、trampoline 叶、文本、trap-context 帧）。
/// Drop 递归归还全部页表帧。
#[derive(Debug)]
pub struct Durable {
    pub(super) root: TableNode,
    pub(super) maps: Vec<Map>,
}

impl Durable {
    pub(super) fn new() -> Result<Self, MapError> {
        Ok(Self {
            root: TableNode::root()?,
            maps: Vec::new(),
        })
    }

    /// 逐帧装叶子 PTE（**放**）：VA 连续推进、PA 取每帧自身（物理可断）。
    ///
    /// 与 [`unmap_frames`](Self::unmap_frames)（**收**，按区间）成对：放按帧
    /// （attach 持帧切片）、收按区间（teardown 不知帧清单）。簿记由调用方完成，
    /// 本方法只装 PTE——与 `TableNode::map`（物理连续版）同族。
    pub(super) fn map_frames(
        &mut self,
        va: VirtAddr,
        frames: &[Frame],
        flags: PteFlags,
    ) -> Result<(), MapError> {
        for (i, frame) in frames.iter().enumerate() {
            let pa = PhysAddr::from_raw(frame.as_ptr() as usize);
            let addr = va + i * PAGE_SIZE;
            self.root.map(addr, pa, PAGE_SIZE, flags)?;
        }
        Ok(())
    }

    /// 摘 `[va, va+size)` 叶 PTE 并回收变空的中间表（unmap/munmap/dealloc/回滚共用）。
    ///
    /// `map_frames` 的反向（收）；回收 = 自底向上判空（512 项全无效）即摘除子树、
    /// 帧当场归还——树与 PTE 同源（`TableNode::reclaim` 先清 PTE 再摘子节点）。
    /// 层级与 VA 掩码按当前模式几何派生。
    pub(crate) fn unmap_frames(&mut self, va: VirtAddr, size: usize) {
        if size == 0 {
            return;
        }
        for i in 0..size.div_ceil(PAGE_SIZE) {
            self.root.unmap(va + i * PAGE_SIZE);
        }
        // 模式 VA 宽度掩码（剥符号扩展：内核半区 VA 的高位）
        let geo = mode::geometry(mode::mode());
        let mask = (1usize << geo.va_bits) - 1;
        let end = va.as_usize().saturating_add(size);
        self.root.reclaim(
            (geo.levels - 1) as usize,
            0,
            va.as_usize() & mask,
            end & mask,
        );
    }

    /// 查询覆盖 `vaddr` 的常数映射。
    pub(super) fn resolve_ref(&self, vaddr: VirtAddr) -> Option<&Map> {
        self.maps.iter().rev().find(|m| m.contains(vaddr))
    }

    /// 查询覆盖 `vaddr` 的常数映射（可变，缺页注入帧用）。
    pub(super) fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        self.maps.iter_mut().rev().find(|m| m.contains(vaddr))
    }
}

