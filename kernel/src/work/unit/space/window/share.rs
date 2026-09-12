// ShareWindow — 共享/可复用懒匿名区域（窗口适配层：user 段上的懒映射策略）。
//
// mmap 取懒匿名段（`reserve(Lazy)`，帧空 → 不物化；触碰经缺页物化零页）。
// munmap 区间精确释放：还段 + 统一拆除（摘 map + 清已触页 PTE，O(触页数)）。
// 与堆/栈同池取段（user 段），无方向分区。

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::map::Pending;
use super::super::salvage::Span;
use super::super::{SegmentKind, Space};

/// 共享懒窗口（零状态策略）。
pub(crate) struct ShareWindow;

impl ShareWindow {
    /// 懒匿名映射（mmap）：user 段取段 + `reserve(Lazy)`（帧空 → 懒）。
    /// 触碰经既有缺页懒分配零页帧（materialize → Lazy 分支）。
    /// 返回 Span（含 seg/va/size，pa=None）。
    ///
    /// # Errors
    ///
    /// - `NotAligned` — size 未页对齐或为零。
    /// - `OutOfMemory` — user 段空隙不足。
    pub(crate) fn mmap(space: &Space, size: usize) -> Result<Span, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        // U 位经 Space::pte_policy 单一出口（U 态需 U；S 态 SUM=0 不得带 U）。
        let flags = space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
        space.with(|inner| {
            let va = inner.allocate(SegmentKind::NonKernel, size)?;
            if let Err(e) = inner.map(va, size, flags, Some(Pending::Lazy)) {
                // reserve 未落任何 PTE/帧；段退回
                inner.deallocate(SegmentKind::NonKernel, va.as_usize(), size);
                return Err(e);
            }
            Ok(Span::new(SegmentKind::NonKernel, va, size, None))
        })
    }

    /// 释放 mmap 区域：精确匹配**校验**段 → 统一拆除（摘 map 帧交料箱 + 清已
    /// 触页 PTE + 回收中间表）→ 结清（跨核清退到齐后才还段与帧）。返回是否
    /// 找到并释放。
    ///
    /// **懒区只有已触页有 PTE/帧**：PTE 清理按已物化帧数逐页走（O(触页数)，
    /// 非 O(段大小)）——1 TiB 级区域不可逐页扫全段。这一步在 `Space::unmap` 内部
    /// 由 map 的 `is_materialized` 决定，与下面这条收敛无关。
    ///
    /// # 本方法不再自己拆装
    ///
    /// 与 `HeapWindow::deallocate` 一样，此前它把 `Space::release` 的整段流程
    /// （`holds` + `unmap` + `take_span` + `reclaim`）复制了一遍，逐字同构——
    /// 同一件事三份实现、三套失败语义。收敛后只剩一层：校验与还原走
    /// [`Space::release_addr`]，拆装与结清只有 `Space::release` 一处。
    ///
    /// 语义不变：该区间不是本段的已分配块 → `false`（状态未动）。
    pub(crate) fn munmap(space: &Space, addr: VirtAddr, size: usize) -> bool {
        space.release_addr(SegmentKind::NonKernel, addr, size)
    }
}
