// ShareWindow — 共享/可复用懒匿名区域（窗口适配层：user 段上的懒映射策略）。
//
// mmap 取懒匿名段（Lazy，帧空 → 不物化；触碰经缺页物化零页）。
// munmap 区间精确释放：摘 map + 清已触页 PTE（O(触页数)）+ 归还段。
// 与堆/栈同池取段（user 段），无方向分区。

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::{Seg, Space};
use super::super::core::Span;
use super::super::map::{Map, Pending};

/// 共享懒窗口（零状态策略）。
pub(crate) struct ShareWindow;

impl ShareWindow {
    /// 懒匿名映射（mmap）：user 段取段 + Lazy map（帧空 → 懒）。
    /// 触碰经既有缺页懒分配零页帧（page_fault → Lazy 分支）。
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
            let user = inner.user.as_mut().ok_or(MapError::NoRegion)?;
            let base = user.allocate(size).map_err(|_| MapError::OutOfMemory)?;
            let va = VirtAddr::from_raw(base);
            let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
            inner.maps.push(Map::new(
                va,
                size,
                flags,
                Some(Pending::Lazy),
                alloc::vec::Vec::new(),
            ));
            Ok(Span::new(Seg::User, va, size, None))
        })
    }

    /// 释放 mmap 区域：精确匹配摘 map（帧随 drop 归还）+ 清已触页 PTE +
    /// 归还段。返回是否找到并释放。
    ///
    /// **懒区只有已触页有 PTE/帧**：PTE 清理按已物化帧数逐页走（O(触页数)，
    /// 非 O(段大小)）——1 TiB 级区域不可逐页扫全段。中间表回收由
    /// `clear_ptes` 统一做（单次遍历现存树）。
    pub(crate) fn munmap(space: &Space, addr: VirtAddr, size: usize) -> bool {
        space.with_flush(|inner| {
            let Some(user) = inner.user.as_mut() else {
                return false;
            };
            if !user.deallocate(addr.as_usize(), size) {
                return false;
            }
            let end = addr.as_usize().saturating_add(size);
            inner.maps.retain(|m| {
                !(addr.as_usize() < m.va.as_usize().saturating_add(m.size.get())
                    && end > m.va.as_usize())
            });
            inner.clear_ptes(addr, size);
            true
        })
    }
}
