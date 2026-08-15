// Sv39 三级页表结构 — 页表遍历、映射、取消映射、中间表回收
//
// Sv39 地址分解：
//   VA[38:30] → VPN[2] — 根页表 (Level 2, L2) 索引
//   VA[29:21] → VPN[1] — 中间页表 (Level 1, L1) 索引
//   VA[20:12] → VPN[0] — 叶子页表 (Level 0, L0) 索引
//   VA[11:0]            — 页内偏移
//
// 所有权模型（TableNode）：Sv39 要求每级页表恰好一帧（4096 B，对齐 4096），
// PageTable 装不下任何元数据——树状所有权因此放在**帧外**的 TableNode 上：
// `page` 是硬件页（根/中间表），`children` 是子树（槽位 → 节点）。树与 PTE
// 经同一入口维护：walk_mut 建表时同步写 PTE + push 子节点，reclaim 拆表时
// 先清 PTE 再摘子节点——构造上一致，无第二份待同步状态。

use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box};
use fack::prelude::Error;

use crate::memory::allocator::frame::allocator;
use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

use super::{
    addr::{PhysAddr, VirtAddr},
    entry::{PageTableEntry, PteFlags},
};

/// 4 KiB 物理帧 — `Box` 指向分配器管理的页面，Drop 归还 frame 池。
///
/// 仅用于**数据页**（`Map::frames`）；页表页用 `TableNode::page`
/// （`Box<PageTable>`，同为 4096 B / 4096 对齐）——类型即语义，两种帧各归其位。
pub(crate) type Frame = Box<[u8; PAGE_SIZE], &'static dyn Allocator>;

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
    /// 虚拟地址不在任何已注册的 Map 内。
    #[error("virtual address not in any declared map")]
    NoRegion,
    /// DRAM 恒等映射越过用户栈窗口（内存配置非法）。
    #[error("DRAM identity map overlaps the user stack window")]
    DramOverlap,
}

/// Sv39 页表 — 512 条目 × 8 字节 = 4 KiB，对齐到页边界。
///
/// 硬件结构，恰好一帧：不承载任何所有权元数据（见 [`TableNode`]）。
/// 不实现 `Clone` / `Copy`：4 KiB 的隐式复制是错误源。
/// `entries` 字段公开（`pub(crate)`），数组自带 `Index`/slice 操作。
#[repr(C, align(4096))]
#[derive(Debug)]
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
    /// 分配一个零页表帧（根或中间表通用）。
    ///
    /// # Errors
    ///
    /// 物理帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub(crate) fn alloc() -> Result<Box<PageTable, &'static dyn Allocator>, MapError> {
        Box::try_new_in(PageTable::default(), allocator()).map_err(|_| MapError::OutOfMemory)
    }
}

/// 页表所有权节点 — 硬件页 + 子树所有权（堆上，不进帧）。
///
/// - `page`：硬件页表帧（根或中间表；恰好一帧，`repr(C, align(4096))`）
/// - `children`：本表有效子表 `(槽位, 子树)`——槽位 = 对应 VPN 索引
///
/// 树与 PTE 同源：walk_mut 建（写 PTE + push）、reclaim 拆（清 PTE + 摘除），
/// children 恒与 PTE 指向一致。Drop 递归释放全部子树帧（Sv39 三层，深度 ≤ 3）。
#[derive(Debug)]
pub(crate) struct TableNode {
    pub(crate) page: Box<PageTable, &'static dyn Allocator>,
    children: Vec<(usize, TableNode)>,
}

impl TableNode {
    /// 新根节点（根页表帧；satp 写入见 [`Self::ppn`]）。
    pub(crate) fn root() -> Result<Self, MapError> {
        Ok(Self {
            page: PageTable::alloc()?,
            children: Vec::new(),
        })
    }

    /// 本节点页表物理页号（根节点写入 satp 用）。
    pub(crate) fn ppn(&self) -> usize {
        Box::as_ptr(&self.page) as usize >> PAGE_SHIFT
    }

    /// 树中节点总数（根 + 全部子孙；debug 统计/自测用）。
    #[cfg(debug_assertions)]
    pub(crate) fn count(&self) -> usize {
        1 + self.children.iter().map(|(_, c)| c.count()).sum::<usize>()
    }

    /// 分配一个零页表子节点。
    fn new_child() -> Result<Self, MapError> {
        Ok(Self {
            page: PageTable::alloc()?,
            children: Vec::new(),
        })
    }

    /// 把子节点页表的 PPN 装入 `pte`（V 位；非叶，非超页）。
    fn set_child_pte(pte: &mut PageTableEntry, child: &Self) {
        let ppn = (Box::as_ptr(&child.page) as usize >> PAGE_SHIFT) as u64;
        pte.set(ppn, PteFlags::V);
    }

    /// Walk to the leaf PTE (mutable)，沿所有权树下钻。
    ///
    /// `alloc`：缺中间表时是否新建（map/缺页用 true；protect 等只读遍历用
    /// false → [`MapError::NotMapped`]）。新建子表**先入树再写 PTE**——PTE
    /// 永不指向未登记的表；树与 PTE 同源，无第二份待同步状态。
    ///
    /// # Errors
    ///
    /// - `OutOfMemory` — `alloc` 为 true 且物理帧耗尽
    /// - `NotMapped` — `alloc` 为 false 且中间表缺失
    pub(crate) fn walk_mut(
        &mut self,
        vaddr: VirtAddr,
        alloc: bool,
    ) -> Result<&mut PageTableEntry, MapError> {
        // Level 2 → Level 1
        let e2 = &mut self.page.entries[vaddr.vpn(2)];
        if !e2.is_valid() {
            if !alloc {
                return Err(MapError::NotMapped);
            }
            let child = Self::new_child()?;
            self.children.push((vaddr.vpn(2), child));
            let child = self.children.last().expect("just pushed");
            Self::set_child_pte(e2, &child.1);
        }
        let l1node = self
            .children
            .iter_mut()
            .find(|(s, _)| *s == vaddr.vpn(2))
            .expect("child exists (PTE ↔ tree invariant)");
        let l1node = &mut l1node.1;

        // Level 1 → Level 0
        let e1 = &mut l1node.page.entries[vaddr.vpn(1)];
        if !e1.is_valid() {
            if !alloc {
                return Err(MapError::NotMapped);
            }
            let child = Self::new_child()?;
            l1node.children.push((vaddr.vpn(1), child));
            let child = l1node.children.last().expect("just pushed");
            Self::set_child_pte(e1, &child.1);
        }
        let l0node = l1node
            .children
            .iter_mut()
            .find(|(s, _)| *s == vaddr.vpn(1))
            .expect("child exists (PTE ↔ tree invariant)");
        let l0node = &mut l0node.1;

        Ok(&mut l0node.page.entries[vaddr.vpn(0)])
    }

    /// Walk to the leaf PTE read-only，沿所有权树下钻（无裸指针解引用）。
    ///
    /// # Errors
    ///
    /// 中间表或叶 PTE 无效时返回 [`MapError::NotMapped`]（与
    /// [`Self::walk_mut`] 的 alloc=false 语义一致）。
    pub(crate) fn walk_ref(&self, vaddr: VirtAddr) -> Result<(PhysAddr, PteFlags), MapError> {
        let e2 = &self.page.entries[vaddr.vpn(2)];
        if !e2.is_valid() || e2.is_leaf() {
            return Err(MapError::NotMapped);
        }
        let l1node = self
            .children
            .iter()
            .find(|(s, _)| *s == vaddr.vpn(2))
            .ok_or(MapError::NotMapped)?;

        let e1 = &l1node.1.page.entries[vaddr.vpn(1)];
        if !e1.is_valid() || e1.is_leaf() {
            return Err(MapError::NotMapped);
        }
        let l0node = l1node
            .1
            .children
            .iter()
            .find(|(s, _)| *s == vaddr.vpn(1))
            .ok_or(MapError::NotMapped)?;

        let leaf = &l0node.1.page.entries[vaddr.vpn(0)];
        if leaf.is_valid() && leaf.is_leaf() {
            Ok((PhysAddr::from_raw(leaf.paddr() as usize), leaf.flags()))
        } else {
            Err(MapError::NotMapped)
        }
    }

    /// 映射 `size` 字节（n 页）从 `vaddr` 到 `paddr` 的连续区域。
    ///
    /// 按需分配中间表（树 + PTE 同源维护）。
    ///
    /// # 调用约定
    ///
    /// - `vaddr` 必须按 4 KiB 页对齐（`offset() == 0`）
    /// - `paddr` 必须按 4 KiB 页对齐（`is_aligned() == true`）
    /// - `size` 必须是 `PAGE_SIZE` 的整数倍
    ///
    /// # Errors
    ///
    /// - `NotAligned` — 地址或大小未按 4 KiB 对齐。
    /// - `AlreadyMapped` — 任一虚拟地址已被映射。
    /// - `OutOfMemory` — 物理帧耗尽。
    pub(crate) fn map(
        &mut self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }

        let pages = size / PAGE_SIZE;
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let pa = paddr + i * PAGE_SIZE;
            let leaf = self.walk_mut(va, true)?;
            if leaf.is_valid() {
                return Err(MapError::AlreadyMapped);
            }
            let ppn = (pa.as_usize() >> PAGE_SHIFT) as u64;
            leaf.set(ppn, flags | PteFlags::V);
        }
        Ok(())
    }

    /// 清一个叶 PTE（不回收中间表；回收见 [`Self::reclaim`]）。
    ///
    /// 复用 [`Self::walk_mut`]（alloc=false）：中间表缺失时返回 NotMapped，
    /// 与本无映射一致，直接跳过。
    pub(crate) fn unmap(&mut self, vaddr: VirtAddr) {
        if let Ok(leaf) = self.walk_mut(vaddr, false) {
            leaf.clear();
        }
    }

    /// 回收 `[start, end)` 范围内变空的中间表（自底向上），返回本节点是否已全空。
    ///
    /// - `level`：本节点层级（根 = 2，逐层递减；叶层 0 无子节点，直接判空）
    /// - `node_va`：本节点槽 0 覆盖的虚拟地址（39 位空间，调用方先掩码）
    /// - `start` / `end`：unmap 范围（39 位掩码，`[start, end)`）
    ///
    /// 只下钻与范围相交的槽位；子节点全空时**先清本层 PTE 再摘除**（drop 归还
    /// 帧）。范围不相交的子树不会被触及（树中节点只在创建时带有效项、只在变空
    /// 时被摘除——未触及即非空）。root 永不摘——调用方忽略返回值。
    pub(crate) fn reclaim(
        &mut self,
        level: usize,
        node_va: usize,
        start: usize,
        end: usize,
    ) -> bool {
        if level > 0 {
            let span = 1usize << (12 + 9 * level);
            let node_end = node_va.saturating_add(span << 9);
            if end <= node_va || start >= node_end {
                return false; // 范围不相交：本节点未被触及，非空
            }
            let shift = 12 + 9 * level;
            let first = if start > node_va {
                (start - node_va) >> shift
            } else {
                0
            };
            let last = if end < node_end {
                (end - node_va - 1) >> shift
            } else {
                511
            };
            // 先收集待摘槽位（借用：迭代 children 时不能同时移除）
            let mut remove: Vec<usize> = Vec::new();
            for (i, (slot, child)) in self.children.iter_mut().enumerate() {
                if *slot < first || *slot > last {
                    continue;
                }
                let child_va = node_va + (*slot << shift);
                if child.reclaim(level - 1, child_va, start, end) {
                    self.page.entries[*slot].clear();
                    remove.push(i);
                }
            }
            // 倒序 swap_remove：先摘高索引，低索引不受移位影响
            for i in remove.into_iter().rev() {
                self.children.swap_remove(i);
            }
        }
        self.page.entries.iter().all(|e| !e.is_valid())
    }
}
