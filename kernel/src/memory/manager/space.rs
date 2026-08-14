// 地址空间 — MMU 子系统的核心抽象
//
// AddressSpace 拥有一个 Sv39 根页表与全部自有物理帧（frames Vec），提供虚拟→物理
// 映射、权限管理、地址翻译等高层操作。路线 1 后用户空间不共享内核映射——trampoline
// 叶 PTE 只映射不拥有，其余帧全归本空间所有，Drop 遍历回收。

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::AtomicUsize;

use crate::{
    lock::RelLock,
    memory::{
        PAGE_SIZE,
        allocator::frame::{self, Frame, FrameAllocator},
    },
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
    pub base: usize,
    pub edge: usize,
    pub flags: PteFlags,
    pub kind: RegionKind,
}

/// Sv39 虚拟地址空间。
///
/// 拥有根页表与**全部自有物理帧**。路线 1 后用户空间不再复制/共享内核映射
/// （`from_kernel` 只映射 trampoline 叶 PTE——帧归内核、不拥有——与 trap-context
/// 帧——自有）。`frames` 记录自有帧（中间表 + trap-context + 用户数据页），
/// Drop 自动归还；根表由 `root` 字段直接持有（Box 自动 drop）。
///
/// # Concurrency
///
/// 全部可变状态（`SpaceInner`：regions / frames / heap）收进一把 [`RelLock`]，
/// 替换此前的 4 个 `RefCell`。`RelLock` 跨 hart 真自旋互斥、同 hart 可重入——
/// 多核下两个 hart 共享同一空间做 map/unmap/缺页时互斥；同 hart 持锁期间
/// 同步缺页（异步中断不受 SIE 屏蔽）可重入。**约定**：每个公开方法锁恰好
/// 一次，内部直接操作 `root_mut()` + `inner`，不重入——重入时若两个 guard
/// 同时 DerefMut 会构成 `&mut` 别名（UB）。
///
/// 页表树读写与 `SpaceInner` 数据共享同一把锁：`translate` 读页表、`map`/
/// `unmap` 写页表，都要持锁互斥（页表修改跨核可见性由锁的 Release/Acquire 保证）。
///
/// # Drop
///
/// `root`（Box）与 `frames`（Vec<Frame>）随字段自动 drop 归还 frame 池，
/// 无需遍历页表树、无需手写 deallocate。
pub struct AddressSpace {
    /// 根页表 — 强类型 `Box<PageTable>`：读路径（translate）走 `as_ref()` 免 unsafe；
    /// 写路径（map/unmap/protect）持 inner 锁期间经 `root_mut()` 拿 `&mut`。
    root: Box<PageTable, &'static FrameAllocator>,
    /// 全部可变状态（regions / frames / heap）——一把可重入锁保护。
    inner: RelLock<SpaceInner>,
    /// 本空间的 ASID（satp.ASID 字段，16 位）。0 = 内核空间（KERNEL_SPACE /
    /// 空闲任务）；任务空间经 [`from_kernel`] 独立分配，Drop 时释放。每任务
    /// 独立 ASID 使 TLB 按空间隔离，切换/页表修改只刷本 ASID 条目。
    asid: usize,
    /// 线程栈窗口分配器 — 窗口号单调递增不回收（虚拟空间足够，~5 万线程/空间）。
    /// 窗口 n 的栈 VA = [`crate::memory::manager::TASK_STACK_BASE`] + n*(栈大小+守护页)。
    pub(crate) stack_windows: AtomicUsize,
    /// 本空间 trap-context 帧物理地址（映射于 TRAP_CONTEXT VA，`from_kernel` 分配）。
    /// spawn 据此写初始帧；内核在 KERNEL_SPACE 恒等访问。
    trap_cx_pa: usize,
}

/// 地址空间的可变状态——由 [`AddressSpace::inner`] 这把 [`RelLock`] 保护。
struct SpaceInner {
    /// 虚拟内存区域表（声明 + 查询，缺页/堆操作据此解析权限与 kind）。
    regions: Vec<Region>,
    /// 本空间**拥有**的全部 4 KiB 帧（中间表 + trap-context + 用户数据页），
    /// Drop 逐个归还。**不含**：根表（`root` 字段持有）、非拥有叶帧（UART MMIO、
    /// trampoline 叶——映射不拥有）、任务栈帧（16 KiB，归 Task/reclaim 分管）。
    frames: Vec<Frame>,
    /// 用户堆分配游标 — map syscall 分配 VA 的单调游标（初始 = [`crate::memory::manager::USER_HEAP_BASE`]）。
    heap_next: usize,
    /// 用户堆已分配块表 `(va, size)` — unmap syscall 释放时精确匹配查表。
    heap_blocks: Vec<(usize, usize)>,
}

impl SpaceInner {
    fn new() -> Self {
        Self {
            regions: Vec::new(),
            frames: Vec::new(),
            heap_next: crate::memory::manager::USER_HEAP_BASE,
            heap_blocks: Vec::new(),
        }
    }

    /// 查询虚拟地址所属的 Region（内部版，调用者须持锁）。
    fn resolve(&self, vaddr: VirtAddr) -> Option<Region> {
        let addr = vaddr.as_usize();
        let idx = self.regions.partition_point(|r| r.base <= addr);
        if idx == 0 {
            return None;
        }
        let region = self.regions[idx - 1];
        if addr < region.edge {
            Some(region)
        } else {
            None
        }
    }
}

// SAFETY: 全部可变状态由 `RelLock` 互斥（跨 hart 自旋）；页表树读写与 `SpaceInner`
// 共享同一把锁。`asid` / `stack_windows` / `trap_cx_pa` / `root` 分配后不可变。
unsafe impl Send for AddressSpace {}
unsafe impl Sync for AddressSpace {}

impl AddressSpace {
    /// 取根页表的可变引用（写路径用）。
    ///
    /// # Safety
    ///
    /// 必须在持有 [`self.inner`](Self::inner) 锁期间调用——锁保证与 `translate`
    /// 等读路径、与跨 hart 写路径互斥。`&self` 拿 `&mut PageTable` 是共享空间 +
    /// 锁的手写不变量（rCore 的 `get_pte_array()` 同款），raw 指针解引用固有。
    unsafe fn root_mut(&self) -> &'static mut PageTable {
        unsafe { &mut *Box::as_ptr(&self.root).cast_mut() }
    }

    /// 页表读翻译（内部版，调用者须持锁，与 map/unmap 写互斥）。
    fn translate_inner(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.root.as_ref().walk_ref(vaddr).ok()
    }

    // ── 生命周期 ──────────────────────────────────────────────

    /// 创建一个全新的空地址空间，分配根页表帧。
    ///
    /// # Errors
    ///
    /// 物理帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub fn new() -> Result<Self, MapError> {
        let root = PageTable::allocate()?;
        Ok(Self {
            root,
            inner: RelLock::new(SpaceInner::new()),
            asid: 0, // 内核空间：ASID 0 保留
            stack_windows: AtomicUsize::new(0),
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
    pub fn from_kernel() -> Result<Self, MapError> {
        let mut space = Self::new()?;
        space.asid = super::asid::allocate(); // 每任务独立 ASID（1..=65535）

        // 读内核空间的 trampoline 叶 PTE 与 trap-context 帧 PA（KERNEL_SPACE 只读）
        let (tramp_pa, tramp_flags, kernel_trap_cx_pa) = {
            let guard = KERNEL_SPACE.lock();
            let ks = guard.as_ref().ok_or(MapError::NotMapped)?;
            let (tramp_pa, tramp_flags) = ks
                .root
                .as_ref()
                .walk_ref(VirtAddr::from_raw(crate::memory::manager::TRAMPOLINE))?;
            let (kpa, _) = ks
                .root
                .as_ref()
                .walk_ref(VirtAddr::from_raw(crate::memory::manager::TRAP_CONTEXT))?;
            (tramp_pa, tramp_flags, kpa.as_usize())
        };

        // trampoline 叶（帧归内核，不入 frames）
        space.map(
            VirtAddr::from_raw(crate::memory::manager::TRAMPOLINE),
            tramp_pa,
            crate::memory::PAGE_SIZE,
            tramp_flags,
        )?;

        // trap-context 帧：分配 + 拷贝内核元数据 + 映射（自有，入 frames）
        let trap_cx = frame::zeroed_frame().map_err(|_| MapError::OutOfMemory)?;
        let trap_cx_pa = trap_cx.as_ptr() as usize;
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
        )?;
        space.inner.lock().frames.push(trap_cx);
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
        crate::memory::manager::TASK_STACK_BASE
            + slot * (crate::memory::manager::TASK_STACK_SIZE + PAGE_SIZE)
    }

    /// 用户堆分配：`size` 向上页对齐后从堆游标单调分配 VA，逐页从 frame
    /// 分配器取物理页并映射到用户区（U|R|W），块表记录。返回分配 VA。
    ///
    /// 堆区固定 [`crate::memory::manager::USER_HEAP_BASE`] 起 64MiB；游标越界 →
    /// [`MapError::OutOfMemory`]。立即分配（非懒分配）：教学简化，页表与
    /// 物理页当场就位，用户访问不再缺页。
    pub(crate) fn heap_allocate(&self, size: usize) -> Result<usize, MapError> {
        let size = size.next_multiple_of(crate::memory::PAGE_SIZE);
        let mut inner = self.inner.lock();
        let base = inner.heap_next;
        let end = base + size;
        if end > crate::memory::manager::USER_HEAP_BASE + crate::memory::manager::USER_HEAP_SIZE {
            return Err(MapError::OutOfMemory); // 堆区耗尽
        }
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        for i in 0..size / crate::memory::PAGE_SIZE {
            let page = frame::zeroed_frame().map_err(|_| MapError::OutOfMemory)?;
            let pa = page.as_ptr() as usize;
            inner.frames.push(page);
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe {
                self.root_mut().map(
                    VirtAddr::from_raw(base + i * crate::memory::PAGE_SIZE),
                    PhysAddr::from_raw(pa),
                    crate::memory::PAGE_SIZE,
                    flags,
                    &mut inner.frames,
                )?;
            }
        }
        inner.heap_blocks.push((base, size));
        inner.heap_next = end;
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.asid);
        }
        Ok(base)
    }

    /// 用户堆释放：块表精确匹配 `(addr, size)` 后 unmap 并归还物理页。
    ///
    /// 持锁一次性完成：逐页 translate 取物理帧 → retain 从 frames 移除（Box Drop
    /// 归还 frame 池）→ 清叶子 PTE → 删 Region → 刷 TLB。返回是否找到并释放。
    pub(crate) fn heap_deallocate(&self, addr: usize, size: usize) -> bool {
        let mut inner = self.inner.lock();
        let Some(idx) = inner
            .heap_blocks
            .iter()
            .position(|&(va, sz)| va == addr && sz == size)
        else {
            return false;
        };
        let (va, sz) = inner.heap_blocks.remove(idx);
        for i in 0..sz / crate::memory::PAGE_SIZE {
            let v = VirtAddr::from_raw(va + i * crate::memory::PAGE_SIZE);
            if let Some((pa, _)) = self.translate_inner(v) {
                // retain 丢弃的 Frame 由 Drop 归还 frame 池
                inner
                    .frames
                    .retain(|f| f.as_ptr() as usize != pa.as_usize());
            }
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe { self.root_mut().unmap(v) };
        }
        inner
            .regions
            .retain(|r| !(va < r.edge && (va + sz) > r.base));
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.asid);
        }
        true
    }

    /// 映射 `size` 字节虚拟地址到物理地址（唯一公共映射入口）。
    ///
    /// 纯页表操作：仅安装 PTE，不注册 Region。按需分配中间页表——新分配的
    /// 中间表帧收集进 [`frames`](SpaceInner::frames)（所有权驱动回收）。
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
    ) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        // SAFETY: 持 inner 锁期间修改页表，与 translate/unmap 互斥。
        unsafe {
            self.root_mut()
                .map(vaddr, paddr, size, flags, &mut inner.frames)?;
        }
        drop(inner);
        // 按本空间 ASID 局部刷：只失效本地址空间的旧条目，其它任务 TLB 保留。
        // SAFETY: S-mode 下 sfence.vma 恒合法。
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

        let mut inner = self.inner.lock();
        // 页表侧：逐页清叶子 PTE
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe { self.root_mut().unmap(vaddr + i * PAGE_SIZE) };
        }
        // Region 侧：删重叠记录
        inner.regions.retain(|r| !(start < r.edge && end > r.base));
        drop(inner);

        // SAFETY: S-mode 下 sfence.vma 恒合法。
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
        let _guard = self.inner.lock();
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            // SAFETY: 持 inner 锁期间修改页表；None = 不分配中间表。
            let leaf = unsafe { self.root_mut().walk_mut(va, None)? };
            leaf.set_flags(flags | PteFlags::V);
        }
        drop(_guard);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
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
    ) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        // 前提：Region 已存在
        inner.resolve(vaddr).ok_or(MapError::NoRegion)?;

        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let page = frame::zeroed_frame().map_err(|_| MapError::OutOfMemory)?;
            let pa = page.as_ptr() as usize;
            inner.frames.push(page);
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe {
                self.root_mut().map(
                    va,
                    PhysAddr::from_raw(pa),
                    PAGE_SIZE,
                    flags,
                    &mut inner.frames,
                )?;
            }
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.asid);
        }
        Ok(())
    }

    // ── 查询 ──────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位。
    ///
    /// 未映射时返回 `None`。持锁与 map/unmap 的页表写互斥。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        let _guard = self.inner.lock();
        self.translate_inner(vaddr)
    }

    /// 返回根页表页号（写入 `satp` 用）。
    pub fn root(&self) -> usize {
        Box::as_ptr(&self.root) as usize >> crate::memory::PAGE_SHIFT
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

        let mut inner = self.inner.lock();
        let overlap = inner.regions.iter().any(|r| start < r.edge && end > r.base);
        if overlap {
            return Err(MapError::AlreadyMapped);
        }

        let region = Region {
            base: start,
            edge: end,
            flags,
            kind,
        };
        let idx = inner.regions.partition_point(|r| r.base < start);
        inner.regions.insert(idx, region);
        Ok(())
    }

    /// 查询虚拟地址所属的 Region。
    ///
    /// 返回 `Option<Region>`（Copy）而非引用：Region 表在锁内，
    /// borrow 不能跨锁返回引用。
    pub fn resolve(&self, vaddr: VirtAddr) -> Option<Region> {
        self.inner.lock().resolve(vaddr)
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        // 先释放本空间的 ASID：`free` 内部会 sfence 该 ASID 的 TLB 残留条目
        // （ASID 可能被后续任务复用，旧条目须失效）。0 = 内核空间，不参与分配。
        if self.asid != 0 {
            super::asid::deallocate(self.asid);
        }
        // `root`（Box<PageTable>）与 `frames`（Vec<Frame>）随字段自动 drop 归还
        // frame 池——所有权驱动，无遍历页表树、无手写 deallocate。
    }
}

impl AddressSpace {
    /// 登记一个本空间拥有的帧（中间表 / 数据帧 / trap-context 帧）——供内核
    /// 空间 init 等把已分配的帧纳入所有权，Box Drop 时自动归还 frame 池。
    pub(crate) fn track_frame(&self, frame: Frame) {
        self.inner.lock().frames.push(frame);
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
pub(crate) static KERNEL_SPACE: RelLock<Option<AddressSpace>> = RelLock::new(None);

/// 获取内核地址空间的锁保护引用。
pub(crate) fn kernel_space() -> crate::lock::reentrant::RelLockGuard<'static, Option<AddressSpace>>
{
    KERNEL_SPACE.lock()
}

/// 内核 trap-context 帧物理地址（`init()` 写入；trap::init 写元数据、idle_frame
/// 写空闲合成帧、from_kernel 从它拷内核切换信息）。
pub static KERNEL_TRAP_CX_PA: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// 内核 trap-context 帧物理地址。
pub fn kernel_trap_cx_pa() -> usize {
    KERNEL_TRAP_CX_PA.load(core::sync::atomic::Ordering::Relaxed)
}

/// 内核地址空间 satp token（写入 satp 用）。KERNEL_SPACE 根表在 init 后固定。
pub fn kernel_token() -> usize {
    let guard = KERNEL_SPACE.lock();
    let ks = guard
        .as_ref()
        .expect("kernel address space not initialized");
    crate::memory::manager::satp_token(ks.root(), 0)
}
