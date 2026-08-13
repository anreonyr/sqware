// 地址空间 — MMU 子系统的核心抽象
//
// AddressSpace 拥有一个 Sv39 根页表与全部自有物理帧（frames Vec），提供虚拟→物理
// 映射、权限管理、地址翻译等高层操作。路线 1 后用户空间不共享内核映射——trampoline
// 叶 PTE 只映射不拥有，其余帧全归本空间所有，Drop 遍历回收。

use core::alloc::Allocator;
use core::alloc::Layout;
use core::cell::RefCell;
use core::ptr::NonNull;
use core::sync::atomic::AtomicUsize;

use alloc::vec::Vec;

use crate::{
    lock::RelLock,
    memory::{PAGE_SIZE, platform},
};

use super::{
    addr::{PhysAddr, VirtAddr},
    entry::PteFlags,
    flush_asid,
    table::{MapError, PageTable},
};

/// 虚拟内存区域（Region）类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// 匿名映射 — 缺页时分配零页
    Anonymous,
    /// 预留区域 — 不可访问，缺页时返回错误
    #[allow(dead_code)] // fault.rs 处理其缺页语义；当前无 Reserved 区域实例
    Reserved,
}

/// 虚拟内存区域 — 连续虚拟地址范围
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub flags: PteFlags,
    pub kind: RegionKind,
}

/// Sv39 虚拟地址空间。
///
/// 拥有根页表与**全部自有物理帧**。路线 1 后用户空间不再复制/共享内核映射
/// （`from_kernel` 只映射 trampoline 叶 PTE——帧归内核、不拥有——与 trap-context
/// 帧——自有）。`frames` 记录所有自有帧（根表 + 中间表 + trap-context + 用户数据页），
/// Drop 遍历归还——无跨空间共享帧，无需 skip/COW。
///
/// # Concurrency
///
/// 单 hart 下所有映射/Region 操作经 `&self` + `RefCell` 内部可变（`map` 更新
/// `frames`、`unmap` 改 Region 表）——空间可被多个线程以 `Arc` 共享，线程创建/
/// 缺页时仍能映射。borrow 短暂、无嵌套重入，嵌套 `borrow_mut` 会 panic（防御）。
/// 跨 hart 需外部互斥（尚未多核）。
///
/// # Drop
///
/// 遍历 [`frames`](Self::frames) 把全部自有帧归还页分配器（所有权驱动，无遍历页表树）。
pub struct AddressSpace {
    root: NonNull<PageTable>,
    /// 虚拟内存区域表（RefCell：空间经 Arc 共享后映射操作走 `&self`，
    /// 单 hart 顺序访问 + borrow 短暂，无并发 borrow）。
    regions: RefCell<Vec<Region>>,
    /// 本空间**拥有**的全部 4 KiB 帧（根表 + 中间表 + trap-context + 用户数据页），
    /// Drop 逐个归还。**不含**：非拥有叶帧（UART MMIO、trampoline 叶——映射不拥有）、
    /// 任务栈帧（16 KiB，归 Task/reclaim 分管）。RefCell 同 regions。
    frames: RefCell<Vec<NonNull<u8>>>,
    /// 本空间的 ASID（satp.ASID 字段，16 位）。0 = 内核空间（KERNEL_SPACE /
    /// 空闲任务）；任务空间经 [`from_kernel`] 独立分配，Drop 时释放。每任务
    /// 独立 ASID 使 TLB 按空间隔离，切换/页表修改只刷本 ASID 条目。
    asid: usize,
    /// 线程栈窗口分配器 — 窗口号单调递增不回收（虚拟空间足够，~5 万线程/空间）。
    /// 窗口 n 的栈 VA = [`crate::memory::manager::TASK_STACK_BASE`] + n*(栈大小+守护页)。
    pub(crate) stack_windows: AtomicUsize,
    /// 用户堆分配游标 — map syscall 分配 VA 的单调游标（初始 = [`crate::memory::manager::USER_HEAP_BASE`]）。
    /// 线程共享空间天然共享堆；不回收（教学简化，64MiB 足够）。
    heap_next: RefCell<usize>,
    /// 用户堆已分配块表 `(va, size)` — unmap syscall 释放时精确匹配查表。
    heap_blocks: RefCell<Vec<(usize, usize)>>,
    /// 本空间 trap-context 帧物理地址（映射于 TRAP_CONTEXT VA，`from_kernel` 分配）。
    /// spawn 据此写初始帧；内核在 KERNEL_SPACE 恒等访问。
    trap_cx_pa: usize,
}

// SAFETY: 单 hart 内核，映射/Region 操作顺序执行（RefCell borrow 短暂）；
// 跨 hart 并发访问需外部互斥（多核 TODO）。
unsafe impl Send for AddressSpace {}
unsafe impl Sync for AddressSpace {}

impl AddressSpace {
    /// # Safety
    ///
    /// 地址空间必须已初始化（root 指向有效的 PageTable）。
    unsafe fn root_mut(&self) -> &'static mut PageTable {
        unsafe { &mut *self.root.as_ptr() }
    }

    /// # Safety
    ///
    /// 地址空间必须已初始化（root 指向有效的 PageTable）。
    unsafe fn root_ref(&self) -> &'static PageTable {
        unsafe { &*self.root.as_ptr() }
    }

    // ── 生命周期 ──────────────────────────────────────────────

    /// 创建一个全新的空地址空间，分配根页表帧。
    ///
    /// # Errors
    ///
    /// 物理帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub fn new(alloc: &dyn Allocator) -> Result<Self, MapError> {
        let root = PageTable::allocate(alloc)?;
        let mut frames = Vec::new();
        frames.push(root.cast::<u8>()); // 根表自有
        Ok(Self {
            root,
            regions: RefCell::new(Vec::new()),
            frames: RefCell::new(frames),
            asid: 0, // 内核空间：ASID 0 保留
            stack_windows: AtomicUsize::new(0),
            heap_next: RefCell::new(crate::memory::manager::USER_HEAP_BASE),
            heap_blocks: RefCell::new(Vec::new()),
            trap_cx_pa: 0,
        })
    }

    /// 从内核地址空间创建用户空间（**全私有**，路线 1）。
    ///
    /// 不复制内核半区映射——用户页表只含用户映射 + 两处固定 VA：
    /// - trampoline 叶 PTE 复制（`TRAMPOLINE` VA → 内核 trampoline 物理页，**不拥有**）；
    /// - trap-context 帧（`TRAP_CONTEXT` VA，**自有**，入 frames）。
    ///
    /// 内核切换元数据（kernel_satp / kernel_sp / trap_handler / trap_stack_corrupt）
    /// 从内核 trap-context 帧（trap::init 已写入）复制，`self_pa` 设为本帧物理地址。
    ///
    /// # Errors
    ///
    /// 页表/数据帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub fn from_kernel(alloc: &dyn Allocator) -> Result<Self, MapError> {
        let mut space = Self::new(alloc)?;
        space.asid = super::asid::allocate(); // 每任务独立 ASID（1..=65535）

        // 读内核空间的 trampoline 叶 PTE 与 trap-context 帧 PA（KERNEL_SPACE 只读）
        let (tramp_pa, tramp_flags, kernel_trap_cx_pa) = {
            let guard = KERNEL_SPACE.lock();
            let ks = guard.as_ref().ok_or(MapError::NotMapped)?;
            let (tramp_pa, tramp_flags) = unsafe {
                ks.root_ref().walk_ref(VirtAddr::from_raw(crate::memory::manager::TRAMPOLINE))?
            };
            let (kpa, _) = unsafe {
                ks.root_ref().walk_ref(VirtAddr::from_raw(crate::memory::manager::TRAP_CONTEXT))?
            };
            (tramp_pa, tramp_flags, kpa.as_usize())
        };

        // trampoline 叶（帧归内核，不 track）
        space.map(
            VirtAddr::from_raw(crate::memory::manager::TRAMPOLINE),
            tramp_pa,
            crate::memory::PAGE_SIZE,
            tramp_flags,
            alloc,
        )?;

        // trap-context 帧：分配 + 拷贝内核元数据 + 映射（自有，入 frames）
        let trap_cx = crate::memory::allocator::page::allocator()
            .allocate(
                Layout::from_size_align(crate::memory::PAGE_SIZE, crate::memory::PAGE_SIZE).unwrap(),
            )
            .map_err(|_| MapError::OutOfMemory)?;
        let trap_cx_pa = trap_cx.as_ptr() as *mut u8 as usize;
        // SAFETY: 内核帧 PA 恒等可读（trap::init 已写入元数据）；新帧独占。
        unsafe {
            let ktc = kernel_trap_cx_pa as *const crate::context::TrapContext;
            let ntc = trap_cx_pa as *mut crate::context::TrapContext;
            (*ntc).kernel_satp = (*ktc).kernel_satp;
            (*ntc).kernel_sp = (*ktc).kernel_sp;
            (*ntc).trap_handler = (*ktc).trap_handler;
            (*ntc).trap_stack_corrupt = (*ktc).trap_stack_corrupt;
            (*ntc).self_pa = trap_cx_pa;
        }
        space.map(
            VirtAddr::from_raw(crate::memory::manager::TRAP_CONTEXT),
            PhysAddr::from_raw(trap_cx_pa),
            crate::memory::PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
            alloc,
        )?;
        space.track_frame(trap_cx);
        space.trap_cx_pa = trap_cx_pa;
        Ok(space)
    }

    // ── 映射操作 ──────────────────────────────────────────────

    /// 线程栈窗口 VA：窗口 `slot` 的栈区 = `TASK_STACK_BASE + slot*(栈+守护页)`。
    ///
    /// 窗口 0 与现状固定窗口一致；窗口 n 的守护页 [va-4K, va) 恰为窗口 n-1
    /// 栈顶——窗口间无缝，栈溢出仍触发守护页缺页。窗口号经
    /// [`stack_windows`](Self::stack_windows) 分配，单调递增不回收
    /// （1GiB 用户区支持约 5 万窗口）。
    pub(crate) fn stack_window_va(&self, slot: usize) -> usize {
        // 防溢出：slot 超出用户区容量时 wrap 会撞进代码页（debug 构建暴露）
        debug_assert!(slot < 0x4000_0000 / (crate::memory::manager::TASK_STACK_SIZE + PAGE_SIZE));
        crate::memory::manager::TASK_STACK_BASE + slot * (crate::memory::manager::TASK_STACK_SIZE + PAGE_SIZE)
    }

    /// 用户堆分配：`size` 向上页对齐后从堆游标单调分配 VA，逐页从 frame
    /// 分配器取物理页并映射到用户区（U|R|W），块表记录。返回分配 VA。
    ///
    /// 堆区固定 [`crate::memory::manager::USER_HEAP_BASE`] 起 64MiB；游标越界 →
    /// [`MapError::OutOfMemory`]。立即分配（非懒分配）：教学简化，页表与
    /// 物理页当场就位，用户访问不再缺页。
    pub(crate) fn heap_allocate(
        &self,
        size: usize,
        alloc: &dyn Allocator,
    ) -> Result<usize, MapError> {
        let size = size.next_multiple_of(crate::memory::PAGE_SIZE);
        let mut next = self.heap_next.borrow_mut();
        let base = *next;
        let end = base + size;
        if end > crate::memory::manager::USER_HEAP_BASE + crate::memory::manager::USER_HEAP_SIZE {
            return Err(MapError::OutOfMemory); // 堆区耗尽
        }
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        for i in 0..size / crate::memory::PAGE_SIZE {
            let page = crate::memory::allocator::frame::allocator()
                .allocate(
                    Layout::from_size_align(crate::memory::PAGE_SIZE, crate::memory::PAGE_SIZE)
                        .unwrap(),
                )
                .map_err(|_| MapError::OutOfMemory)?;
            self.track_frame(page);
            let pa = page.as_ptr() as *mut u8 as usize;
            self.map(
                VirtAddr::from_raw(base + i * crate::memory::PAGE_SIZE),
                PhysAddr::from_raw(pa),
                crate::memory::PAGE_SIZE,
                flags,
                alloc,
            )?;
        }
        self.heap_blocks.borrow_mut().push((base, size));
        *next = end;
        Ok(base)
    }

    /// 用户堆释放：块表精确匹配 `(addr, size)` 后 unmap 并归还物理页。
    ///
    /// 归还顺序：translate 逐页取物理帧 → frame 分配器 deallocate → unmap
    /// （页表清理 + 按空间 ASID 局部刷 TLB）。返回是否找到并释放。
    pub(crate) fn heap_deallocate(&self, addr: usize, size: usize) -> bool {
        let mut blocks = self.heap_blocks.borrow_mut();
        let Some(idx) = blocks.iter().position(|&(va, sz)| va == addr && sz == size) else {
            return false;
        };
        let (va, sz) = blocks.remove(idx);
        drop(blocks); // 释放 borrow，避免与下方 translate 的字段 borrow 冲突
        for i in 0..sz / crate::memory::PAGE_SIZE {
            let v = VirtAddr::from_raw(va + i * crate::memory::PAGE_SIZE);
            if let Some((pa, _)) = self.translate(v) {
                // SAFETY: 本块物理页由 heap_alloc 从 frame 分配器分配，layout 一致
                unsafe {
                    crate::memory::allocator::frame::allocator().deallocate(
                        NonNull::new(pa.as_usize() as *mut u8).unwrap(),
                        Layout::from_size_align(crate::memory::PAGE_SIZE, crate::memory::PAGE_SIZE)
                            .unwrap(),
                    );
                }
                self.untrack_frame(pa.as_usize()); // 已归还，Drop 不再重复释放
            }
        }
        self.unmap(VirtAddr::from_raw(va), sz);
        true
    }

    /// 映射 `size` 字节虚拟地址到物理地址（唯一公共映射入口）。
    ///
    /// 纯页表操作：仅安装 PTE，不注册 Region。按需分配中间页表——新分配的
    /// 中间表帧收集进 [`frames`](Self::frames)（所有权驱动回收）。
    ///
    /// **vaddr、paddr、size 必须全部按 [`PAGE_SIZE`] 对齐**。
    /// 非对齐大小的调用方（如 MMIO 设备映射）须自行向上取整。
    ///
    /// 路线 1 后无共享子树：本空间所有页表帧私有，`map` 直接写叶子，无需 COW。
    ///
    /// # Errors
    ///
    /// 参见 [`PageTable::map`]。
    pub fn map(
        &self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
        alloc: &dyn Allocator,
    ) -> Result<(), MapError> {
        let mut new_tables = Vec::new();
        // SAFETY: 地址空间已初始化，map 只修改页表
        unsafe { self.root_mut().map(vaddr, paddr, size, flags, alloc, &mut new_tables)? };
        self.frames.borrow_mut().extend(new_tables);
        // 按本空间 ASID 局部刷：只失效本地址空间的旧条目，其它任务 TLB 保留。
        // SAFETY: executed in S-mode; sfence.vma is always legal.
        unsafe {
            flush_asid(self.asid);
        }
        Ok(())
    }

    /// 取消映射一段虚拟地址并移除其 Region 记录（ecall munmap 后端）。
    ///
    /// 页表侧逐页清叶子 PTE（惰性策略，不释放中间页表）；Region 侧按重叠
    /// 删除与 `[start, start+size)` 相交的所有记录。`vaddr`/`size` 不要求
    /// 页对齐（向上取整语义与 POSIX munmap 一致）。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        let start = vaddr.as_usize();
        let end = start + size;

        // 页表侧：逐页清叶子 PTE
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            // SAFETY: 地址空间已初始化，unmap 只清零叶子 PTE
            unsafe { self.root_mut().unmap(vaddr + i * PAGE_SIZE) };
        }

        // Region 侧：删重叠记录
        self.regions
            .borrow_mut()
            .retain(|r| !(start < r.end && end > r.start));

        // SAFETY: executed in S-mode; sfence.vma is always legal.
        unsafe {
            flush_asid(self.asid);
        }
    }

    /// 修改已映射区域的保护标志。
    ///
    /// 单次遍历，不分配中间表——叶子 PTE 不存在则返回错误。
    ///
    /// # Errors
    ///
    /// 任一页的叶子 PTE 不存在时返回 [`MapError::NotMapped`]。
    #[allow(dead_code)] // ecall mprotect 后端预留
    pub fn protect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            // SAFETY: 地址空间已初始化，protect 只修改已有叶子 PTE 标志位
            let leaf = unsafe { self.root_mut().walk_mut(va, None, None)? };
            leaf.set_flags(flags | PteFlags::V);
        }
        // SAFETY: executed in S-mode; sfence.vma is always legal.
        unsafe {
            flush_asid(self.asid);
        }
        Ok(())
    }

    // ── 缺页处理 ──────────────────────────────────────────────

    /// 缺页处理：查 Region → 分配零页 → 映射。
    ///
    /// 从 frame 分配器逐页取物理帧，清零后映射到 `vaddr` 起始的连续区间。
    /// 必须在目标地址已注册 Anonymous Region 时调用。
    ///
    /// # Errors
    ///
    /// - [`MapError::NoRegion`] — 地址不在任何 Region 内
    /// - [`MapError::OutOfMemory`] — 物理帧耗尽
    pub fn page_fault(
        &self,
        vaddr: VirtAddr,
        size: usize,
        flags: PteFlags,
        alloc: &dyn Allocator,
    ) -> Result<(), MapError> {
        // 前提：Region 已存在
        self.resolve(vaddr).ok_or(MapError::NoRegion)?;

        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let layout = core::alloc::Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
            let page = alloc.allocate(layout).map_err(|_| MapError::OutOfMemory)?;
            // SAFETY: 分配器刚给的独占页，清零以保证安全
            unsafe {
                core::ptr::write_bytes(page.as_ptr() as *mut u8, 0, PAGE_SIZE);
            }
            self.track_frame(page);
            let pa = PhysAddr::from_raw(page.as_ptr() as *mut u8 as usize);
            self.map(va, pa, PAGE_SIZE, flags, alloc)?;
        }
        Ok(())
    }

    // ── 查询 ──────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位。
    ///
    /// 未映射时返回 `None`。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        // SAFETY: address space is initialized; read-only traversal.
        unsafe { self.root_ref().walk_ref(vaddr).ok() }
    }

    /// 返回根页表页号（写入 `satp` 用）。
    pub fn root(&self) -> usize {
        self.root.as_ptr() as usize >> crate::memory::PAGE_SHIFT
    }

    /// 返回本空间的 ASID（写入 `satp.ASID` 用；0 = 内核空间）。
    pub fn asid(&self) -> usize {
        self.asid
    }

    // ── Region 管理 ───────────────────────────────────────────

    /// 声明一段预留虚拟区域：首次访问触发缺页时按 `kind` 分配
    /// （Anonymous → 分配零页，见 [`page_fault`](Self::page_fault)）。
    ///
    /// `start` 和 `size` 必须 `PAGE_SIZE` 对齐，不得与已有 Region 重叠。
    /// 与 [`resolve`](Self::resolve)（查询）配对；删除随
    /// [`unmap`](Self::unmap) 原子完成（清页表 + 移除声明）。
    pub fn declare(
        &self,
        start: usize,
        size: usize,
        flags: PteFlags,
        kind: RegionKind,
    ) -> Result<(), MapError> {
        if !start.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let end = start + size;

        // 按起始地址排序检查重叠（borrow 短作用域，避免与下方 borrow_mut 冲突）
        let overlap = self
            .regions
            .borrow()
            .iter()
            .any(|r| start < r.end && end > r.start);
        if overlap {
            return Err(MapError::AlreadyMapped);
        }

        let region = Region {
            start,
            end,
            flags,
            kind,
        };
        let idx = self.regions.borrow().partition_point(|r| r.start < start);
        self.regions.borrow_mut().insert(idx, region);
        Ok(())
    }

    /// 查询虚拟地址所属的 Region。
    ///
    /// 返回 `Option<Region>`（Copy）而非引用：Region 表为 `RefCell`，
    /// borrow 不能跨语句返回引用。
    pub fn resolve(&self, vaddr: VirtAddr) -> Option<Region> {
        let addr = vaddr.as_usize();
        let regions = self.regions.borrow();
        let idx = regions.partition_point(|r| r.start <= addr);
        if idx == 0 {
            return None;
        }
        let region = regions[idx - 1];
        if addr < region.end {
            Some(region)
        } else {
            None
        }
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        // 先释放本空间的 ASID：`free` 内部会 sfence 该 ASID 的 TLB 残留条目
        // （ASID 可能被后续任务复用，旧条目须失效）。0 = 内核空间，不参与分配。
        if self.asid != 0 {
            super::asid::deallocate(self.asid);
        }
        // 所有权驱动回收：遍历 frames 归还全部自有帧（根表 + 中间表 +
        // trap-context + 用户数据页）。无共享帧，无需遍历页表树。
        // SAFETY: 帧均由本空间从 frame/page 分配器分配，归还 layout 一致（4 KiB）。
        let layout = Layout::from_size_align(crate::memory::PAGE_SIZE, crate::memory::PAGE_SIZE).unwrap();
        for ptr in self.frames.borrow_mut().drain(..) {
            unsafe {
                crate::memory::allocator::frame::allocator().deallocate(ptr, layout);
            }
        }
    }
}

impl AddressSpace {
    /// 把本空间**拥有**的数据帧纳入 [`frames`](Self::frames)（Drop 统一归还）。
    ///
    /// 只收录自有帧（heap / page_fault / ELF 装载分配的数据页、trap-context 帧）；
    /// 非拥有叶帧（UART MMIO、trampoline 叶）与任务栈（reclaim 分管）不入。
    pub(crate) fn track_frame(&self, frame: NonNull<[u8]>) {
        self.frames.borrow_mut().push(frame.cast::<u8>());
    }

    /// 从 [`frames`](Self::frames) 移除已归还的帧（heap_deallocate 用）——
    /// 避免 Drop 二次归还。
    fn untrack_frame(&self, pa: usize) {
        self.frames.borrow_mut().retain(|&p| p.addr().get() != pa);
    }

    /// 本空间 trap-context 帧物理地址（`from_kernel` 分配，spawn 写初始帧用）。
    pub(crate) fn trap_context_pa(&self) -> usize {
        self.trap_cx_pa
    }
}

// ── 内核地址空间 ─────────────────────────────────────────────

/// 内核地址空间。`memory::init()` 创建并写入，此后只读访问。
///
/// 用 RelLock（可重入锁）：持有此锁期间若触发缺页，缺页处理器（trap.rs）
/// 会在同一 hart 上再次获取它——RelLock 允许同 hart 重入，避免自旋死锁；
/// 不同 hart 之间仍互斥。
static KERNEL_SPACE: RelLock<Option<AddressSpace>> = RelLock::new(None);

/// 获取内核地址空间的锁保护引用。
pub fn kernel_space() -> crate::lock::reentrant::RelLockGuard<'static, Option<AddressSpace>> {
    KERNEL_SPACE.lock()
}

/// 内核 trap-context 帧物理地址（`init()` 写入；trap::init 写元数据、idle_frame
/// 写空闲合成帧、from_kernel 从它拷内核切换信息）。
static KERNEL_TRAP_CX_PA: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// 内核 trap-context 帧物理地址。
pub fn kernel_trap_cx_pa() -> usize {
    KERNEL_TRAP_CX_PA.load(core::sync::atomic::Ordering::Relaxed)
}

/// 内核地址空间 satp token（写入 satp 用）。KERNEL_SPACE 根表在 init 后固定。
pub fn kernel_token() -> usize {
    let guard = KERNEL_SPACE.lock();
    let ks = guard.as_ref().expect("kernel address space not initialized");
    crate::memory::manager::satp_token(ks.root(), 0)
}

/// 初始化 MMU：创建内核地址空间，identity-map DRAM 和 MMIO，启用 Sv39 分页。
///
/// 必须在 `memory::allocator::init()` 之后、在驱动程序 MMIO 访问之前调用。
///
/// # Safety
///
/// 写入 `satp` 后会立即启用分页。调用者需确保此时所有存活的指针
/// （栈、代码、数据段）都已 identity-mapped。
/// # Errors
///
/// - [`MapError::OutOfMemory`] — 物理帧不足以分配根页表或中间页表。
pub unsafe fn init() -> Result<(), MapError> {
    unsafe {
        let alloc = crate::memory::allocator::page::allocator();
        let cfg = platform::get();

        // 任务栈窗口 TASK_STACK_BASE=0xC0000000 的前提：DRAM 必须 < 1 GiB。
        // 否则窗口落入 DRAM 恒等映射区，任务栈覆盖真实内存而非专用窗口。
        assert!(
            cfg.dram_size <= 0x4000_0000,
            "task stack window (TASK_STACK_BASE) requires DRAM < 1 GiB (got {:#x})",
            cfg.dram_size
        );

        // 1. 创建内核地址空间
        let kernel_space = AddressSpace::new(alloc)?;

        // 2. Identity-map DRAM
        let ram_flags = PteFlags::V
            | PteFlags::R
            | PteFlags::W
            | PteFlags::X
            | PteFlags::A
            | PteFlags::D
            | PteFlags::G;

        kernel_space.map(
            VirtAddr::from_raw(cfg.dram_base),
            PhysAddr::from_raw(cfg.dram_base),
            cfg.dram_size,
            ram_flags,
            alloc,
        )?;

        // 3. 建立内核高半区映射（为 S-mode 切换做准备）
        let kernel_va_base = VirtAddr::from_raw(VirtAddr::KERNEL_BASE + cfg.dram_base);
        kernel_space.map(
            kernel_va_base,
            PhysAddr::from_raw(cfg.dram_base),
            cfg.dram_size,
            ram_flags,
            alloc,
        )?;

        // 4. 映射 trap trampoline 页（内核自有帧）：所有空间以 TRAMPOLINE VA
        //    映射同一物理页，`stvec` 指向它。G 位：内容不可变，不被 ASID 局部
        //    sfence 刷掉也安全。
        let tramp_flags = PteFlags::V
            | PteFlags::R
            | PteFlags::X
            | PteFlags::A
            | PteFlags::D
            | PteFlags::G;
        kernel_space.map(
            VirtAddr::from_raw(crate::memory::manager::TRAMPOLINE),
            PhysAddr::from_raw(crate::runtime::trampoline::trampoline_pa()),
            crate::memory::PAGE_SIZE,
            tramp_flags,
            alloc,
        )?;

        // 5. 内核 trap-context 帧：映射于 TRAP_CONTEXT（内核自身 trap 用；
        //    元数据字段由 trap::init 写入），PA 存入 KERNEL_TRAP_CX_PA。
        let ktc = crate::memory::allocator::page::allocator()
            .allocate(
                Layout::from_size_align(crate::memory::PAGE_SIZE, crate::memory::PAGE_SIZE).unwrap(),
            )
            .map_err(|_| MapError::OutOfMemory)?;
        let ktc_pa = ktc.as_ptr() as *mut u8 as usize;
        kernel_space.map(
            VirtAddr::from_raw(crate::memory::manager::TRAP_CONTEXT),
            PhysAddr::from_raw(ktc_pa),
            crate::memory::PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
            alloc,
        )?;
        kernel_space.track_frame(ktc);
        KERNEL_TRAP_CX_PA.store(ktc_pa, core::sync::atomic::Ordering::Relaxed);

        // 6. 启用 Sv39 分页
        riscv::register::satp::set(riscv::register::satp::Mode::Sv39, 0, kernel_space.root());

        // 7. 刷新 TLB
        flush_asid(0);

        // 8. 保存内核地址空间
        KERNEL_SPACE.lock().replace(kernel_space);

        Ok(())
    }
}
