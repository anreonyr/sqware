// 多级页表结构 — 页表遍历、映射、取消映射、中间表回收（层级随运行模式）。
//
// 地址分解（层级 L = 模式几何；每级 9 位 VPN，页内偏移 12 位）：
//   VA[12+9·L−1 : 12+9·(L−1)] → VPN[L−1] — 根页表索引（顶层随模式：Sv39=2 / Sv48=3 / Sv57=4）
//   ...                               → 中间页表索引
//   VA[20:12]                        → VPN[0] — 叶子页表索引
//   VA[11:0]                         → 页内偏移
//
// 所有权模型（TableNode）：每级页表恰好一帧（4096 B，对齐 4096），
// PageTable 装不下任何元数据——树状所有权因此放在**帧外**的 TableNode 上：
// `page` 是硬件页（根/中间表），`children` 是子树（槽位 → 节点）。树与 PTE
// 经同一入口维护：walk_mut 建表时同步写 PTE + push 子节点，reclaim 拆表时
// 先清 PTE 再摘子节点——构造上一致，无第二份待同步状态。

use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box, sync::Arc};
use fack::prelude::Error;

use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

use super::{
    addr::{PhysAddr, VirtAddr},
    entry::{PageTableEntry, PteFlags},
};

/// 4 KiB 物理帧 — `Box` 指向分配器管理的页面，Drop 归还 frame 池。
///
/// 仅用于**数据页**；页表页用 `TableNode::page`（`Box<PageTable>`，同为
/// 4096 B / 4096 对齐）——类型即语义，两种帧各归其位。
pub(crate) type Frame = Box<[u8; PAGE_SIZE], &'static dyn Allocator>;

/// 数据页的共享 / 私有身份 —— COW 共享帧。
///
/// Shared = 共享只读视图：多条 PTE / 多个空间引用同一物理页，写缺页触发
/// `own` 分裂。Arc 带帧分配器（配 Arc::new_in），底层仍从 buddy 分配
/// / 归还——不串物理帧池。
/// Owned = 私有可写视图（原 Frame 独占 Box 语义；Shared→Owned 必经一次拷贝）。
#[derive(Debug)]
pub(crate) enum FrameState {
    Shared(Arc<[u8; PAGE_SIZE], &'static dyn Allocator>),
    Owned(Frame),
}

impl FrameState {
    /// 页物理地址（Shared/Owned 都指向一页；恒等映射下指针即 PA）。
    /// 仅 audit（精确比对 PTE↔帧）使用。
    #[cfg(feature = "audit")]
    pub(crate) fn pa(&self) -> PhysAddr {
        let p = match self {
            FrameState::Shared(a) => Arc::as_ptr(a) as usize,
            FrameState::Owned(b) => b.as_ptr() as usize,
        };
        PhysAddr::from_raw(p)
    }
}

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

/// 页表页 — 512 条目 × 8 字节 = 4 KiB，对齐到页边界（三类模式同宽）。
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
    pub(crate) fn new() -> Result<Box<PageTable, &'static dyn Allocator>, MapError> {
        // 同 [u8;4096] 教训：PageTable::default() 按值 4 KiB 会在调用栈上物化约
        // 16 KiB 栈帧——任务栈上建中间表同样击穿（风暴 UAF 同源）。走标准原语
        // Box::try_new_zeroed_in（allocate_zeroed，栈上不物化）。
        // 类别 = Table：页表页（root 与 walk_mut 子表）——关机与内核根表 walk
        // 计数核对（audit::check_baseline ③）。
        Ok(crate::tag!(Table, unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        }))
    }
}

/// 页表所有权节点 — 硬件页 + 子树所有权（堆上，不进帧）。
///
/// - `page`：硬件页表帧（根或中间表；恰好一帧，`repr(C, align(4096))`）
/// - `children`：本表有效子表 `(槽位, 子树)`——槽位 = 对应 VPN 索引
///
/// 树与 PTE 同源：walk_mut 建（写 PTE + push）、reclaim 拆（清 PTE + 摘除），
/// children 恒与 PTE 指向一致。Drop 递归释放全部子树帧（深度 = 模式层级 ≤ 5）。
#[derive(Debug)]
pub(crate) struct TableNode {
    pub(crate) page: Box<PageTable, &'static dyn Allocator>,
    children: Vec<(usize, TableNode)>,
}

impl TableNode {
    /// 新根节点（根页表帧；satp 写入见 [`Self::ppn`]）。
    pub(crate) fn root() -> Result<Self, MapError> {
        Ok(Self {
            page: PageTable::new()?,
            children: Vec::new(),
        })
    }

    /// 本节点页表物理页号（根节点写入 satp 用）。
    pub(crate) fn ppn(&self) -> usize {
        Box::as_ptr(&self.page) as usize >> PAGE_SHIFT
    }

    /// 树中节点总数（根 + 全部子孙；health/自测用，debug-only——非审计链）。
    #[cfg(debug_assertions)]
    pub(crate) fn count(&self) -> usize {
        1 + self.children.iter().map(|(_, c)| c.count()).sum::<usize>()
    }

    /// 创建一个零页表子节点 + 把它的 PPN 装入 `pte`（V 位；非叶，非超页）。
    ///
    /// 一步完成"建表 + 装 PTE 引用"——保证新子节点在 tree 入册前 PTE 已 V，
    /// 永不指向未登记的表。返回的 child 由调用方 push 进父的 `children`。
    ///
    /// 命名"leaf"——PTE 视角看这是叶子表节点（被 PTE 指向的下一层表）。
    fn leaf(pte: &mut PageTableEntry) -> Result<Self, MapError> {
        let child = Self {
            page: PageTable::new()?,
            children: Vec::new(),
        };
        let ppn = (Box::as_ptr(&child.page) as usize >> PAGE_SHIFT) as u64;
        pte.set(ppn, PteFlags::V);
        Ok(child)
    }

    /// Walk to the leaf PTE (mutable)，沿所有权树下钻。
    ///
    /// `alloc`：缺中间表时是否新建（map/缺页用 true；mprotect 等只读遍历用
    /// false → [`MapError::NotMapped`]）。新建子表**先入树再写 PTE**——PTE
    /// 永不指向未登记的表；树与 PTE 同源，无第二份待同步状态。
    ///
    /// `levels`：下钻层数。正常路径传当前模式层级（[`super::mode::levels`]）；
    /// 模式探测等显式场景传候选层级。
    ///
    /// # Errors
    ///
    /// - `OutOfMemory` — `alloc` 为 true 且物理帧耗尽
    /// - `NotMapped` — `alloc` 为 false 且中间表缺失
    pub(crate) fn walk_mut(
        &mut self,
        vaddr: VirtAddr,
        alloc: bool,
        levels: usize,
    ) -> Result<&mut PageTableEntry, MapError> {
        let mut node = self;
        for level in (0..levels).rev() {
            let idx = vaddr.vpn(level as u8);
            if level == 0 {
                return Ok(&mut node.page.entries[idx]);
            }
            let valid = node.page.entries[idx].is_valid();
            if !valid {
                if !alloc {
                    return Err(MapError::NotMapped);
                }
                let child = Self::leaf(&mut node.page.entries[idx])?;
                node.children.push((idx, child));
            }
            node = &mut node
                .children
                .iter_mut()
                .find(|(s, _)| *s == idx)
                .expect("child exists (PTE ↔ tree invariant)")
                .1;
        }
        unreachable!("loop covers 0..levels")
    }

    /// 树外裸路径：沿 PTE 链从根帧遍历到 leaf 页（**不依赖本树元数据**——
    /// 入口为根页表物理地址；现场/诊断专用，守卫由调用方提供）。
    ///
    /// 根、每级中间表与 leaf 的 PA 都经 `ok`（守卫）校验后才访问/返回，坏地址的
    /// 裸读 fault 在此拦下。逐级读 PTE：V=0 未映射 → None；R|W|X 置位 = leaf
    /// （返回页基 + flags，支持任意级超页）；否则沿 PTE.PPN 下钻。
    pub(crate) fn walk_raw(
        root: PhysAddr,
        page_va: VirtAddr,
        ok: impl Fn(PhysAddr) -> bool,
    ) -> Option<(PhysAddr, PteFlags)> {
        let levels = super::mode::levels();
        let mut tbl = root;
        if !ok(tbl) {
            return None;
        }
        for level in (0..levels).rev() {
            // SAFETY: tbl 已过 ok 校验（合法可读物理区）；调用方保证只读、无并发写。
            let pte = unsafe {
                *((tbl.as_usize() + page_va.vpn(level as u8) * 8) as *const PageTableEntry)
            };
            if !pte.is_valid() {
                return None;
            }
            if pte.is_leaf() {
                let pa = PhysAddr::from_raw(pte.paddr() as usize);
                return ok(pa).then_some((pa, pte.flags()));
            }
            tbl = PhysAddr::from_raw(pte.paddr() as usize);
            if !ok(tbl) {
                return None;
            }
        }
        None
    }

    /// Walk to the leaf PTE read-only，沿所有权树下钻（无裸指针解引用）。
    ///
    /// 可信任路径（树内）：children 引用链由 Box 持有、借用检查保护——即使
    /// PTE 被软件写坏，也不会裸解引用 PPN（正常路径的内存安全不依赖「树与
    /// PTE 同源」不变量）。树外裸路径（现场/诊断，无树可用）走
    /// [`Self::walk_raw`]（裸 PPN + 调用方守卫）——按依赖分工，互不外包。
    ///
    /// # Errors
    ///
    /// 中间表或叶 PTE 无效（含中间级 leaf 超页）时返回 [`MapError::NotMapped`]。
    pub(crate) fn walk_ref(&self, vaddr: VirtAddr) -> Result<(PhysAddr, PteFlags), MapError> {
        let levels = super::mode::levels();
        let mut node = self;
        for level in (0..levels).rev() {
            let idx = vaddr.vpn(level as u8);
            let e = &node.page.entries[idx];
            if level == 0 {
                return if e.is_valid() && e.is_leaf() {
                    Ok((PhysAddr::from_raw(e.paddr() as usize), e.flags()))
                } else {
                    Err(MapError::NotMapped)
                };
            }
            if !e.is_valid() || e.is_leaf() {
                return Err(MapError::NotMapped);
            }
            node = &node
                .children
                .iter()
                .find(|(s, _)| *s == idx)
                .ok_or(MapError::NotMapped)?
                .1;
        }
        unreachable!("loop covers 0..levels")
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
            let leaf = self.walk_mut(va, true, super::mode::levels())?;
            if leaf.is_valid() {
                return Err(MapError::AlreadyMapped);
            }
            let ppn = (pa.as_usize() >> PAGE_SHIFT) as u64;
            leaf.set(ppn, flags | PteFlags::V);
        }
        Ok(())
    }

    /// 改 `[vaddr, vaddr+size)` 内每页的叶 PTE flags（不动中间表结构）。
    ///
    /// 已触页 / Eager 全物化页：walk 到叶 PTE 翻 flags。
    /// 中间表缺失：walk_mut(false) 返回 NotMapped（错误传播）。
    ///
    /// 簿记同步（Lazy 区未触页改 map.flags）由 `SpaceInner::protect` 处理——本函数
    /// 只动页表。
    pub(crate) fn protect(
        &mut self,
        vaddr: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0 || vaddr.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let pages = size / PAGE_SIZE;
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let leaf = self.walk_mut(va, false, super::mode::levels())?;
            leaf.set_flags(flags);
        }
        Ok(())
    }

    /// 清理 `[vaddr, vaddr+size)` 范围内的叶 PTE + 回收变空的中间表。
    ///
    /// 多页版：逐页 `walk_mut(false)` 清叶（中间表缺失返回 NotMapped = 与无映射
    /// 一致，直接跳过），再自底向上回收变空中间表。一次调用完成完整清理——
    /// 不再需要单独的 `reclaim`。
    pub(crate) fn unmap(&mut self, vaddr: VirtAddr, size: usize) {
        if size == 0 || vaddr.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return;
        }
        let pages = size / PAGE_SIZE;
        // 1. 清叶 PTE
        for i in 0..pages {
            if let Ok(leaf) = self.walk_mut(vaddr + i * PAGE_SIZE, false, super::mode::levels()) {
                leaf.clear();
            }
        }
        // 2. 拆空中间表（自底向上）
        let geo = super::mode::geometry(super::mode::mode());
        let mask = (1usize << geo.va_bits) - 1;
        Self::recycle(
            self,
            (geo.levels - 1) as usize,
            0,
            vaddr.as_usize() & mask,
            (vaddr.as_usize() + size) & mask,
        );
    }

    /// 自底向上回收 `[start, end)` 范围内变空的中间表，返回本节点是否全空。
    ///
    /// 只下钻与范围相交的槽位；子节点全空时**先清本层 PTE 再摘除**（drop 归还
    /// 帧）。范围不相交的子树不会被触及（树中节点只在创建时带有效项、只在变空
    /// 时被摘除——未触及即非空）。root 永不摘——调用方忽略返回值。
    ///
    /// 仅 [`Self::unmap`] 内部调用——不对外暴露。
    fn recycle(&mut self, level: usize, node_va: usize, start: usize, end: usize) -> bool {
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
                if child.recycle(level - 1, child_va, start, end) {
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
