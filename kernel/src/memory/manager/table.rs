// Sv39 三级页表结构 — 页表遍历、映射、取消映射、帧分配/释放
//
// Sv39 地址分解：
//   VA[38:30] → VPN[2] — 根页表 (Level 2, L2) 索引
//   VA[29:21] → VPN[1] — 中间页表 (Level 1, L1) 索引
//   VA[20:12] → VPN[0] — 叶子页表 (Level 0, L0) 索引
//   VA[11:0]            — 页内偏移

use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box};
use fack::prelude::Error;

use crate::memory::allocator::frame::allocator;
use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

use super::{
    addr::{PhysAddr, VirtAddr},
    entry::{PageTableEntry, PteFlags},
};

/// 页表操作错误。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// 物理页帧分配器耗尽。
    #[error("physical frame allocator exhausted")]
    OutOfMemory,
    /// 该虚拟地址已被映射。
    #[error("virtual address already mapped")]
    AlreadyMapped,
    /// 地址未按页对齐。
    #[error("address not page-aligned")]
    NotAligned,
    /// 页表项/中间表不存在。
    #[error("page table entry not mapped")]
    NotMapped,
    /// 虚拟地址不在任何已注册的 Region 内。
    #[error("virtual address not in any declared region")]
    NoRegion,
    /// DRAM 恒等映射越过用户栈窗口（内存配置非法）。
    #[error("DRAM identity map overlaps the user stack window")]
    DramOverlap,
}

/// Sv39 页表 — 512 条目 × 8 字节 = 4 KiB，对齐到页边界。
///
/// 不实现 `Clone` / `Copy`：4 KiB 的隐式复制是错误源。
/// `entries` 字段公开（`pub(crate)`），数组自带 `Index`/`IndexMut`/slice 操作——
/// 无需额外 trait impl。
#[repr(C, align(4096))]
pub(crate) struct PageTable {
    pub(crate) entries: [PageTableEntry; 512],
}

impl Default for PageTable {
    fn default() -> Self {
        Self {
            entries: [PageTableEntry::default(); 512],
        }
    }
}

impl PageTable {
    /// 从分配器分配一个零页表。
    ///
    /// # Errors
    ///
    /// 物理帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub(crate) fn root() -> Result<Box<PageTable, &'static dyn Allocator>, MapError> {
        Box::try_new_in(PageTable::default(), allocator()).map_err(|_| MapError::OutOfMemory)
    }

    /// Walk to the leaf PTE read-only, returning the physical address and flags.
    ///
    /// Returns an error when an intermediate table or the leaf PTE is invalid,
    /// matching the error type used by [`walk_mut`](Self::walk_mut).
    pub(crate) fn walk_ref(&self, vaddr: VirtAddr) -> Result<(PhysAddr, PteFlags), MapError> {
        let l2 = &self.entries[vaddr.vpn(2)];
        if !l2.is_valid() || l2.is_leaf() {
            return Err(MapError::NotMapped);
        }
        // SAFETY: l2 is valid and not a leaf (checked above); paddr() points to a valid PageTable frame.
        let p1 = unsafe { &*(l2.paddr() as *const PageTable) };

        let l1 = &p1.entries[vaddr.vpn(1)];
        if !l1.is_valid() || l1.is_leaf() {
            return Err(MapError::NotMapped);
        }
        // SAFETY: l1 is valid and not a leaf (checked above); paddr() points to a valid PageTable frame.
        let p0 = unsafe { &*(l1.paddr() as *const PageTable) };

        let leaf = &p0.entries[vaddr.vpn(0)];
        if leaf.is_valid() && leaf.is_leaf() {
            Ok((PhysAddr::from_raw(leaf.paddr() as usize), leaf.flags()))
        } else {
            Err(MapError::NotMapped)
        }
    }

    /// Walk to the leaf PTE and return a mutable reference.
    ///
    /// `frames` 控制中间表分配：`Some` 时按需分配，新帧**当场** push 进 `frames`
    /// （所有权在分配点即归 `Space`，无 out-param、无泄漏）；`None` 时
    /// 缺中间表即 [`MapError::NotMapped`]（`protect` 等只读遍历用）。
    ///
    /// # Physical-to-virtual assumption
    ///
    /// This method dereferences intermediate-table physical addresses as
    /// `*mut PageTable`. It assumes all physical memory is identity-mapped.
    /// In debug builds, consider asserting `paddr` is within DRAM bounds.
    ///
    /// # Errors
    ///
    /// - `OutOfMemory` — `frames` is `Some` and physical frames exhausted
    /// - `NotMapped` — `frames` is `None` and an intermediate table is missing
    pub(crate) fn walk_mut(
        &mut self,
        vaddr: VirtAddr,
        mut frames: Option<&mut Vec<Box<[u8; PAGE_SIZE], &'static dyn Allocator>>>,
    ) -> Result<&mut PageTableEntry, MapError> {
        // Level 2 → Level 1
        let l2 = &mut self.entries[vaddr.vpn(2)];
        if !l2.is_valid() {
            install_child(l2, &mut frames)?;
        }
        // SAFETY: l2 is valid (pre-existing or just allocated with V only, hence not a leaf).
        // paddr() points to a valid PageTable frame.
        let p1 = unsafe { &mut *(l2.paddr() as *mut PageTable) };

        // Level 1 → Level 0
        let l1 = &mut p1.entries[vaddr.vpn(1)];
        if !l1.is_valid() {
            install_child(l1, &mut frames)?;
        }
        // SAFETY: l1 is valid (pre-existing or just allocated with V only, hence not a leaf).
        // paddr() points to a valid PageTable frame.
        let p0 = unsafe { &mut *(l1.paddr() as *mut PageTable) };

        Ok(&mut p0.entries[vaddr.vpn(0)])
    }

    /// 映射 `size` 字节（n 页）从 `vaddr` 到 `paddr` 的连续区域。
    ///
    /// 按需分配中间页表节点。
    ///
    /// # 调用约定
    ///
    /// - `vaddr` 必须按 4 KiB 页对齐（`offset() == 0`）
    /// - `paddr` 必须按 4 KiB 页对齐（`is_aligned() == true`）
    /// - `size` 必须是 `PAGE_SIZE` 的整数倍
    ///
    /// 不符合对齐要求的调用返回 [`MapError::NotAligned`]。
    /// 需要自动取整的调用者应自行向上取整（MMIO 设备映射由
    /// 驱动层 `map_mmio` 承担）。
    ///
    /// # Errors
    ///
    /// - [`MapError::NotAligned`] — 地址或大小未按 4 KiB 对齐。
    /// - [`MapError::AlreadyMapped`] — 任一虚拟地址已被映射。
    /// - [`MapError::OutOfMemory`] — 物理帧耗尽。
    pub(crate) fn map(
        &mut self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
        frames: &mut Vec<Box<[u8; PAGE_SIZE], &'static dyn Allocator>>,
    ) -> Result<(), MapError> {
        if vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }

        let pages = size / PAGE_SIZE;
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let pa = paddr + i * PAGE_SIZE;
            let leaf = self.walk_mut(va, Some(frames))?;
            if leaf.is_valid() {
                return Err(MapError::AlreadyMapped);
            }
            let ppn = (pa.as_usize() >> PAGE_SHIFT) as u64;
            leaf.set(ppn, flags | PteFlags::V);
        }
        Ok(())
    }

    /// 取消映射一个虚拟地址。
    ///
    /// 将叶子 PTE 清零，不释放中间页表节点（惰性策略）。
    ///
    /// 复用 [`walk_mut`](Self::walk_mut)（`frames = None`）：中间表缺失时返回
    /// `NotMapped`，与本无映射一致，直接跳过。
    pub(crate) fn unmap(&mut self, vaddr: VirtAddr) {
        if let Ok(leaf) = self.walk_mut(vaddr, None) {
            leaf.clear();
        }
    }
}

/// 分配一个零帧中间表并安装到 `pte`（`frames` 为 `None` 时视为「不分配」，报 NotMapped）。
///
/// 新帧在分配点即 push 进 `frames`，所有权归 `Space`——即使后续 `map`
/// 因 `AlreadyMapped` 提前返回，帧也不泄漏（Drop 统一归还）。
fn install_child(
    pte: &mut PageTableEntry,
    frames: &mut Option<&mut Vec<Box<[u8; PAGE_SIZE], &'static dyn Allocator>>>,
) -> Result<(), MapError> {
    let frames = frames.as_mut().ok_or(MapError::NotMapped)?;
    let child =
        Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
    let child_pa = PhysAddr::from_raw(child.as_ptr() as usize);
    frames.push(child);
    pte.set((child_pa.as_usize() >> PAGE_SHIFT) as u64, PteFlags::V);
    Ok(())
}
