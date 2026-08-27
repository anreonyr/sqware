// HeapWindow — 用户堆窗口（动态侧：堆 / mmap 懒区）。
//
// 装载期注册 `[image_end, 栈底 = upper() − STACK_WINDOW_SIZE)`，区间分配器 ∝ 存活块。
// 同一窗口方向分区：allocate rise 出块（立即分配：逐页帧 + PTE + 注入）、mmap fall
// 取高位懒匿名段（mmap/munmap：帧空 → 懒，触碰经缺页物化）。护栏事件（fence
// 记账）与 TLB 刷新属空间级事务，由调用方（envcall，经 `Space::with_flush`）负责。

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::allocator::interval::Direction;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::Frame;
use crate::memory::manager::{MapError, mode};

use super::super::durable::Durable;
use super::super::dynamic::Dynamic;
use super::super::map::{Map, MapKind};

/// 堆窗口。
#[derive(Debug)]
pub(crate) struct HeapWindow {
    /// 公共窗口核心（区间分配器 + 子 Map 表）。
    pub(crate) inner: Dynamic,
}

impl HeapWindow {
    /// 装载期注册：`[base, edge)`（通常 `[image_end, upper() − STACK_WINDOW_SIZE)`）。
    /// 调用方保证 base 页对齐、base ≤ edge 且不与已映射区/窗口重叠（见 loader）。
    pub(crate) fn new(base: usize, edge: usize) -> Self {
        Self {
            inner: Dynamic::window(base, edge),
        }
    }

    /// 用户堆分配：窗口 rise 保留页对齐 VA 块，登记空子 Map（Anonymous），逐页从
    /// frame 分配器取物理页映射（U|R|W，**立即分配**非懒）并注入子 Map。返回分配 VA。
    ///
    /// 中途帧耗尽时回滚：清已映射页叶子 + 移除子 Map（帧随 drop 归还）+ VA 块退回
    /// 窗口（中间表帧已由 unmap_frames 回收）。
    ///
    /// # Errors
    ///
    /// 窗口耗尽 / 物理帧耗尽 → [`MapError::OutOfMemory`]。
    pub(crate) fn allocate(
        &mut self,
        durable: &mut Durable,
        size: usize,
    ) -> Result<VirtAddr, MapError> {
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        let va = self
            .inner
            .allocate(size, flags, MapKind::Anonymous, None)?;
        let pages = size / PAGE_SIZE;
        let mut mapped = 0usize;
        while mapped < pages {
            let page: Frame = unsafe {
                Box::try_new_zeroed_in(allocator())
                    .map_err(|_| MapError::OutOfMemory)?
                    .assume_init()
            };
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            let m_va = va + mapped * PAGE_SIZE;
            if durable.root.map(m_va, pa, PAGE_SIZE, flags).is_err() {
                // 回滚：清已映射页叶子并回收中间表 + 移除子 Map（帧随 drop 归还）+ VA 退回窗口
                durable.unmap_frames(va, mapped * PAGE_SIZE);
                self.inner.deallocate(va, size);
                return Err(MapError::OutOfMemory);
            }
            let child = self
                .inner
                .children
                .iter_mut()
                .find(|m| m.va == va)
                .expect("heap child exists");
            child.inject(page);
            mapped += 1;
        }
        Ok(va)
    }

    /// 用户堆释放：窗口按 `(addr, size)` 精确匹配后移除子 Map（帧随 drop 归还）+
    /// 清叶子 PTE（含回收变空的中间表）。返回是否找到并释放（未分配/部分已释放的
    /// 区间返回 false，同旧块表精确匹配语义）。
    pub(crate) fn deallocate(&mut self, durable: &mut Durable, addr: VirtAddr, size: usize) -> bool {
        // 1. 区间精确匹配释放 + 移除子 Map（未分配 → 返回 false）
        if !self.inner.deallocate(addr, size) {
            return false;
        }
        // 2. 清叶子 PTE + 回收变空的中间表（帧已随子 Map 移除 drop 归还）
        durable.unmap_frames(addr, size);
        true
    }

    /// 高位大段懒匿名映射（mmap）：窗口 `fall` 取高位段 + Anonymous 子 Map（帧空 →
    /// 懒）。触碰经既有缺页懒分配零页帧（page_fault → Anonymous 分支）；返回高位 VA
    /// （Sv39 ≈ 254 GiB / Sv48 ≈ 128 TiB / Sv57 ≈ 64 PiB）。与堆共用同一窗口，
    /// 方向分区：堆 rise、mmap fall。
    ///
    /// # Errors
    ///
    /// - `NotAligned` — size 未页对齐或为零。
    /// - `OutOfMemory` — 窗口空隙不足。
    pub(crate) fn mmap(&mut self, size: usize) -> Result<VirtAddr, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let (base, size) = self
            .inner
            .allocator
            .allocate(size, Direction::Fall)
            .map_err(|_| MapError::OutOfMemory)?;
        let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
        self.inner.children.push(Map::new(
            VirtAddr::from_raw(base),
            size,
            flags,
            MapKind::Anonymous,
            Vec::new(),
            None,
        ));
        Ok(VirtAddr::from_raw(base))
    }

    /// 释放 mmap 区域：精确匹配摘块摘子 Map（帧随 drop 归还）+ 清已触页 PTE/中间表。
    ///
    /// **懒区只有已触页有 PTE/帧**：PTE 清理按登记帧逐页走（O(触页数)，非 O(段大小)）
    /// ——1 TiB 级区域不可逐页扫全段。帧按触序登记（须有序触碰；乱序由 audit 现行
    /// 暴露）；中间表回收单走一次（O(现存树)）。
    pub(crate) fn munmap(&mut self, durable: &mut Durable, addr: VirtAddr, size: usize) -> bool {
        // 已触页数（= 已登记帧数；懒触碰有序，帧 i ↔ addr + i·PAGE）
        let mapped = match self.inner.children.iter().find(|m| m.va == addr) {
            Some(c) => c.frames.len(),
            None => return false,
        };
        for i in 0..mapped {
            durable
                .root
                .unmap(VirtAddr::from_raw(addr.as_usize() + i * PAGE_SIZE));
        }
        if !self.inner.deallocate(addr, size) {
            return false;
        }
        // 回收变空的中间表（单次遍历现存树结构）
        let geo = mode::geometry(mode::mode());
        let mask = (1usize << geo.va_bits) - 1;
        let end = addr.as_usize().saturating_add(size);
        let top = (geo.levels - 1) as usize;
        durable
            .root
            .reclaim(top, 0, addr.as_usize() & mask, end & mask);
        true
    }
}