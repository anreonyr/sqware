// HeapWindow — 用户堆（窗口适配层：user 段上的堆领域策略，立即物化）。
//
// allocate 出块：user 段取段 + 逐页物化帧（Eager，立即分配非懒）+ 登记 map。
// deallocate 精确匹配释放：摘 map（帧随 drop 归还）+ 清叶 PTE + 归还段。
// 与栈/懒区同池取段（user 段 lowest first-fit），无方向分区。

use alloc::boxed::Box;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::Frame;

use super::super::{Seg, Space};
use super::super::core::Span;
use super::super::map::Map;

/// 堆窗口（零状态策略）。
pub(crate) struct HeapWindow;

impl HeapWindow {
    /// 用户堆分配：user 段取一页对齐 VA 块，逐页从 frame 分配器取物理页
    /// 映射（U|R|W，**立即分配**非懒）并注入 map。返回 Span。
    ///
    /// 中途帧耗尽时回滚：清已映射页叶 + 摘 map + VA 退回段。
    ///
    /// # Errors
    ///
    /// 段耗尽 / 物理帧耗尽 → [`MapError::OutOfMemory`]。
    pub(crate) fn allocate(space: &Space, size: usize) -> Result<Span, MapError> {
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        space.with_flush(|inner| {
            let user = inner.user.as_mut().ok_or(MapError::NoRegion)?;
            let base = user.allocate(size).map_err(|_| MapError::OutOfMemory)?;
            let va = VirtAddr::from_raw(base);
            // 先登记空 map（堆立即物化：pending None），再逐页装 PTE + 注入
            inner.maps.push(Map::new(
                va,
                size,
                flags,
                None,
                alloc::vec::Vec::new(),
            ));
            let pages = size / PAGE_SIZE;
            let mut mapped = 0usize;
            while mapped < pages {
                // 类别 = Task：用户堆页属任务生命周期——关机必须归零（①）。
                let page: Frame = unsafe {
                    Box::try_new_zeroed_in(frame::tag_task())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                };
                let pa = crate::memory::manager::addr::PhysAddr::from_raw(page.as_ptr() as usize);
                let m_va = va + mapped * PAGE_SIZE;
                if inner.root.map(m_va, pa, PAGE_SIZE, flags).is_err() {
                    // 回滚：清已映射页叶 + 摘 map + VA 退回段
                    for i in 0..mapped {
                        inner.root.unmap(va + i * PAGE_SIZE);
                    }
                    inner.maps.retain(|m| m.va != va);
                    inner.user.as_mut().expect("user exists").deallocate(base, size);
                    return Err(MapError::OutOfMemory);
                }
                // 逐页注入帧（保持「帧 i ↔ va + i·PAGE」不变量）
                inner
                    .maps
                    .iter_mut()
                    .find(|m| m.va == va)
                    .expect("heap map exists")
                    .inject(page);
                mapped += 1;
            }
            Ok(Span::new(Seg::User, va, size, None))
        })
    }

    /// 用户堆释放：分配器按 `(addr, size)` 精确匹配后摘 map（帧随 drop 归还）+
    /// 清叶 PTE（含回收变空的中间表）+ 归还段。返回是否找到并释放。
    pub(crate) fn deallocate(space: &Space, addr: VirtAddr, size: usize) -> bool {
        space.with_flush(|inner| {
            // 1. 精确匹配释放（未分配 → false）
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
            // 2. 清叶 PTE + 回收变空的中间表（帧已随 map 移除 drop 归还）
            inner.clear_ptes(addr, size);
            true
        })
    }
}
