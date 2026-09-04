// ShareWindow — 共享/可复用懒匿名区域（窗口适配层：user 段上的懒映射策略）。
//
// mmap 取懒匿名段（`reserve(Lazy)`，帧空 → 不物化；触碰经缺页物化零页）。
// munmap 区间精确释放：还段 + 统一拆除（摘 map + 清已触页 PTE，O(触页数)）。
// 与堆/栈同池取段（user 段），无方向分区。

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::core::{Salvage, Span};
use super::super::map::Pending;
use super::super::{Seg, Space};

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
        space.with(|inner| {
            let va = inner.allocate(Seg::User, size)?;
            let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
            if let Err(e) = inner.map(va, size, flags, Some(Pending::Lazy)) {
                // reserve 未落任何 PTE/帧；段退回
                inner.deallocate(Seg::User, va.as_usize(), size);
                return Err(e);
            }
            Ok(Span::new(Seg::User, va, size, None))
        })
    }

    /// 释放 mmap 区域：精确匹配**校验**段 → 统一拆除（摘 map 帧交料箱 + 清已
    /// 触页 PTE + 回收中间表）→ 结清（跨核清退到齐后才还段与帧）。返回是否
    /// 找到并释放。
    ///
    /// **懒区只有已触页有 PTE/帧**：PTE 清理按已物化帧数逐页走（O(触页数)，
    /// 非 O(段大小)）——1 TiB 级区域不可逐页扫全段。
    pub(crate) fn munmap(space: &Space, addr: VirtAddr, size: usize) -> bool {
        let mut salvage = Salvage::new();
        let found = space.with_flush(|inner| {
            // 1. 只读校验（未分配 → false，状态未动）
            if !inner.holds(Seg::User, addr.as_usize(), size) {
                return false;
            }
            // 2. 统一拆除（清叶先于摘 map；帧与段一并入箱）
            inner.unmap(addr, size, &mut salvage);
            salvage.take_span(Span::new(Seg::User, addr, size, None));
            true
        });
        if found {
            salvage.reclaim(space).expect("munmap: evict deaf");
        }
        found
    }
}
