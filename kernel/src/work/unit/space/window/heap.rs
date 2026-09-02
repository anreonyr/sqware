// HeapWindow — 用户堆（窗口适配层：user 段上的堆领域策略，立即物化）。
//
// allocate 出块：user 段取段 + `claim`（Eager，立即分配非懒）+ 登记 map。
// deallocate 精确匹配释放：还段 + 统一拆除（摘 map 帧随 drop 归还 + 清叶 PTE）。
// 与栈/懒区同池取段（user 段 lowest first-fit），无方向分区。

use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::core::Span;
use super::super::{Seg, Space};

/// 堆窗口（零状态策略）。
pub(crate) struct HeapWindow;

impl HeapWindow {
    /// 用户堆分配：user 段取一页对齐 VA 块，逐页从 frame 分配器取物理页
    /// 映射（U|R|W，**立即分配**非懒）并注入 map。返回 Span。
    ///
    /// 中途帧耗尽时回滚：`claim` 自清已映射页叶 + 摘 map，本方法退 VA 回段。
    ///
    /// # Errors
    ///
    /// 段耗尽 / 物理帧耗尽 → [`MapError::OutOfMemory`]。
    pub(crate) fn allocate(space: &Space, size: usize) -> Result<Span, MapError> {
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        space.with_flush(|inner| {
            let va = inner.allocate(Seg::User, size)?;
            if let Err(e) = inner.claim_map(va, size, flags) {
                // claim 已自回滚装配；段退回
                inner.deallocate(Seg::User, va.as_usize(), size);
                return Err(e);
            }
            Ok(Span::new(Seg::User, va, size, None))
        })
    }

    /// 用户堆释放：分配器按 `(addr, size)` 精确匹配还段，成功 → 统一拆除
    /// （清叶 PTE + 摘 map，帧随 drop 归还）。返回是否找到并释放。
    pub(crate) fn deallocate(space: &Space, addr: VirtAddr, size: usize) -> bool {
        space.with_flush(|inner| {
            // 1. 精确匹配释放（未分配 → false）
            if !inner.deallocate(Seg::User, addr.as_usize(), size) {
                return false;
            }
            // 2. 统一拆除：清叶（必须先于摘 map——clear 按 map 的
            //    is_materialized 决定 unmap 哪些页）+ 摘 map（帧随 drop 归还）。
            inner.unmap(addr, size);
            true
        })
    }
}
