// 地址空间 — MMU 子系统的核心抽象
//
// Space 拥有一个 Sv39 根页表与全部自有物理帧（frames Vec），提供虚拟→物理
// 映射、权限管理、地址翻译等高层操作。空间种类由 [`SpaceKind`] 显式区分：内核空间
// （ASID 0，全局唯一）与用户空间（独立 ASID），构造统一走 [`SpaceBuilder`]。
// 路线 1 后用户空间不共享内核映射——trampoline 叶 PTE 只映射不拥有，其余帧全归本
// 空间所有，Drop 遍历回收。用户堆与任务栈窗口各持一个通用位图分配器实例
// （[`BitmapAllocator`]，见 `memory::allocator::bitmap`），组合进 [`SpaceInner`]，
// 与页表操作同锁互斥；ASID 空间亦为该分配器的全局实例（见 `memory::manager::asid`）。

use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box};
use core::sync::atomic::Ordering;

use crate::lock::reentrant::RelLockGuard;
use crate::memory::allocator::bitmap::BitmapAllocator;
use crate::memory::allocator::frame::allocator;
use crate::{lock::RelLock, memory::PAGE_SIZE};

use super::{
    addr::{AtomicPhysAddr, PhysAddr, VirtAddr},
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
    ///
    /// 任务栈守护页以 Reserved 登记：栈溢出触碰守护页时，fault.rs 据此
    /// 返回「预留区域访问」而非笼统的「无 Region」。
    Reserved,
}

/// 虚拟内存区域 — 连续虚拟地址范围
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub base: VirtAddr,
    pub edge: VirtAddr,
    pub flags: PteFlags,
    pub kind: RegionKind,
}

/// 空间种类 — 显式区分内核空间与用户空间。
///
/// 内核空间 ASID 恒 0、全局唯一；用户空间各自持有独立 ASID（1..=65535），
/// 构造时经 [`super::asid::allocate`] 分配、`Drop` 释放。
///
/// 用户区布局常量（堆窗口 / 任务栈窗口）收敛进本模块（`USER_HEAP_*` / `TASK_STACK_*`），
/// 由 [`BitmapAllocator`] 实例消费。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    /// 内核空间（ASID 0）。
    Kernel,
    /// 用户空间（独立 ASID）。
    User { asid: usize },
}

impl SpaceKind {
    /// 本空间的 ASID（内核恒 0）。
    pub fn asid(&self) -> usize {
        match self {
            SpaceKind::Kernel => 0,
            SpaceKind::User { asid } => *asid,
        }
    }
}

// ── 地址空间布局常量（原 Heap/Stacks 关联常量提升为模块常量）───────────

/// 内核半区起始虚拟地址（VPN[2] = 256，VA = 2^38 符号扩展）— Sv39 用户/内核分界。
pub(crate) const KERNEL_BASE: VirtAddr = VirtAddr::from_raw(0xFFFF_FFC0_0000_0000);

/// 用户堆窗口基址 — map syscall 的堆区起点。
pub(crate) const USER_HEAP_BASE: VirtAddr = VirtAddr::from_raw(0x2000_0000);
/// 用户堆窗口大小（64 MiB）。
pub(crate) const USER_HEAP_SIZE: usize = 0x40_0000;
/// 任务栈窗口基址（1 GiB 用户区，约 5 万窗口）。
pub(crate) const USER_STACK_BASE: VirtAddr = VirtAddr::from_raw(0xC000_0000);
/// 每个任务栈的大小（字节）。
pub(crate) const TASK_STACK_SIZE: usize = 16384;
/// 栈守护页大小（= 一页）。
pub(crate) const STACK_GUARD_SIZE: usize = PAGE_SIZE;
/// 任务栈窗口大小（1 GiB）。
pub(crate) const TASK_STACK_AREA_SIZE: usize = 0x4000_0000;
/// trap 入口 trampoline 页的固定虚拟地址（Sv39 最高页，L2[511]·L1[511]·L0[511]）。
///
/// 一页含 `__alltraps`（保存帧 + 切 satp）与 `__restore`（切回 + 恢复 + sret），
/// 内核空间与所有用户空间以同一 VA 映射**同一物理帧**。`stvec` 指向此处。
pub(crate) const TRAMPOLINE: VirtAddr = VirtAddr::from_raw(0xFFFF_FFFF_FFFF_F000);

/// 每空间 trap-context 页的固定虚拟地址（trampoline 下方一页，L2[511]·L1[511]·L0[510]）。
///
/// 用户空间把本任务的 [`crate::runtime::context::TrapContext`] 帧映射于此（S 态独占、无 U
/// 位）；内核空间映射自己的帧。`__alltraps` 在用户空间经此 VA 存帧；`__restore`
/// 切回目标空间后经此 VA 恢复。
///
/// 本帧物理地址不冗余存储：需要时经 [`translate`](Self::translate) 查询
/// （帧内 `self_pa` 字段亦自存一份）。
pub(crate) const TRAP_CONTEXT: VirtAddr = VirtAddr::from_raw(0xFFFF_FFFF_FFFF_E000);

// ── 地址空间总览（Sv39）───────────────────────────────────────────
//
// 用户半区（bit 38 = 0，VPN[2] = 0..255）：
//
//   0x0000_0000
//     ┌─────────────────────────────────┐
//     │             保留                │
//     ├─────────────────────────────────┤ 0x2000_0000  USER_HEAP_BASE
//     │  用户堆窗口 64 MiB               │ USER_HEAP_BASE + USER_HEAP_SIZE
//     │ （BitmapAllocator 实例管理）     │
//     ├─────────────────────────────────┤ 0x2040_0000
//     │             保留                │
//     ├─────────────────────────────────┤ 0xC000_0000  USER_STACK_BASE
//     │  任务栈窗口 1 GiB                │ USER_STACK_BASE + TASK_STACK_AREA_SIZE
//     │ （16 KiB 栈 + 守护页，约 5 万）  │
//     ├─────────────────────────────────┤ 0x1_0000_0000（低 4 GiB，设计上限）
//     │             保留（至 2^38）      │
//     └─────────────────────────────────┘
//
// 内核半区（bit 38 = 1，VPN[2] = 256..511，起点 KERNEL_BASE）：
//
//   0xFFFF_FFFF_FFFF_F000 — TRAMPOLINE（页对齐；汇编 LUI 由 TRAP_CONTEXT_LUI 注入）
//   0xFFFF_FFFF_FFFF_E000 — TRAP_CONTEXT（= TRAMPOLINE - PAGE_SIZE，相邻页）
//
// 布局即不变量：以下断言把「对齐 / 相邻 / 不重叠」编译期锁死——
// 改布局必须先改这里（编译器兜底），并同步 link.ld / trampoline 汇编。
const _: () = {
    // 注意：VirtAddr 的 Add/Sub/PartialEq 非 const fn，此处一律用 as_usize() 裸算术。
    assert!(TRAMPOLINE.as_usize() % PAGE_SIZE == 0);
    assert!(TRAP_CONTEXT.as_usize() % PAGE_SIZE == 0);
    assert!(TRAMPOLINE.as_usize() - PAGE_SIZE == TRAP_CONTEXT.as_usize()); // 相邻页（trampoline 收尾依赖）
    assert!(KERNEL_BASE.as_usize() == 0xFFFF_FFC0_0000_0000); // Sv39 内核半区起点（VPN[2] = 256）
    assert!(USER_HEAP_BASE.as_usize() % PAGE_SIZE == 0);
    assert!(USER_HEAP_SIZE % PAGE_SIZE == 0);
    assert!(USER_HEAP_BASE.as_usize() + USER_HEAP_SIZE <= USER_STACK_BASE.as_usize()); // 堆窗口不越过栈窗口
    assert!(USER_STACK_BASE.as_usize() % PAGE_SIZE == 0);
    assert!(TASK_STACK_AREA_SIZE % PAGE_SIZE == 0);
    assert!(TASK_STACK_SIZE % PAGE_SIZE == 0);
    assert!(USER_STACK_BASE.as_usize() + TASK_STACK_AREA_SIZE <= 0x1_0000_0000); // 栈窗口不越出低 4 GiB
};

/// Sv39 虚拟地址空间。
///
/// 拥有根页表与**全部自有物理帧**。路线 1 后用户空间不再复制/共享内核映射
/// （用户空间构建只映射 trampoline 叶 PTE——帧归内核、不拥有——与 trap-context
/// 帧——自有）。`frames` 记录自有帧（中间表 + trap-context + 用户数据页），
/// Drop 自动归还；根表由 `root` 字段直接持有（Box 自动 drop）。
///
/// 空间种类由 [`kind`](Self::kind) 显式区分（见 [`SpaceKind`]）。
///
/// # Concurrency
///
/// 全部可变状态（`SpaceInner`：regions / frames / heap / stacks）收进一把 [`RelLock`]，
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
#[derive(Debug)]
pub struct Space {
    /// 根页表 — 强类型 `Box<PageTable>`：读路径（translate）走 `as_ref()` 免 unsafe；
    /// 写路径（map/unmap/protect）持 inner 锁期间经 `root_mut()` 拿 `&mut`。
    root: Box<PageTable, &'static dyn Allocator>,
    /// 全部可变状态（regions / frames / heap / stacks）——一把可重入锁保护。
    inner: RelLock<SpaceInner>,
    /// 空间种类（内核 / 用户），内嵌 ASID。
    kind: SpaceKind,
}

/// 地址空间的可变状态——由 [`Space::inner`] 这把 [`RelLock`] 保护。
///
/// 只做组合：`regions`（VMA 表）与 `frames`（自有帧）是页表/缺页的核心状态；
/// 用户堆与任务栈窗口各持一个 [`BitmapAllocator`] 实例（只算虚拟地址区间、
/// 不碰页表），与映射操作同锁互斥（锁约定见 [`Space`]）。
#[derive(Debug)]
struct SpaceInner {
    /// 虚拟内存区域表（声明 + 查询，缺页/堆操作据此解析权限与 kind）。
    regions: Vec<Region>,
    /// 本空间**拥有**的全部 4 KiB 帧（中间表 + trap-context + 用户数据页），
    /// Drop 逐个归还。**不含**：根表（`root` 字段持有）、非拥有叶帧（UART MMIO、
    /// trampoline 叶——映射不拥有）、任务栈帧（16 KiB，归 Task/reclaim 分管）。
    frames: Vec<Box<[u8; PAGE_SIZE], &'static dyn Allocator>>,
    /// 用户堆窗口位图实例（[`BitmapAllocator`]，随本锁互斥；first-fit + 释放复用）。
    heap: BitmapAllocator,
    /// 任务栈窗口位图实例（[`BitmapAllocator`]，随本锁互斥；窗口可回收复用）。
    stacks: BitmapAllocator,
}

impl SpaceInner {
    fn new() -> Self {
        Self {
            regions: Vec::new(),
            frames: Vec::new(),
            heap: BitmapAllocator::new(
                USER_HEAP_BASE.as_usize(),
                (USER_HEAP_BASE + USER_HEAP_SIZE).as_usize(),
                PAGE_SIZE,
            ),
            stacks: BitmapAllocator::new(
                USER_STACK_BASE.as_usize(),
                (USER_STACK_BASE + TASK_STACK_AREA_SIZE).as_usize(),
                PAGE_SIZE,
            ),
        }
    }

    /// 查询虚拟地址所属的 Region（内部版，调用者须持锁）。
    fn resolve(&self, vaddr: VirtAddr) -> Option<Region> {
        let idx = self.regions.partition_point(|r| r.base <= vaddr);
        if idx == 0 {
            return None;
        }
        let region = self.regions[idx - 1];
        if vaddr < region.edge {
            Some(region)
        } else {
            None
        }
    }

    /// 登记 Region（内部版，调用者须持锁；与 [`Space::declare`] 配对）。
    ///
    /// `start`/`size` 必须页对齐，不得与已有 Region 重叠；按 `base` 有序插入。
    fn declare(
        &mut self,
        start: VirtAddr,
        size: usize,
        flags: PteFlags,
        kind: RegionKind,
    ) -> Result<(), MapError> {
        if !start.as_usize().is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let end = start + size;
        if self.regions.iter().any(|r| start < r.edge && end > r.base) {
            return Err(MapError::AlreadyMapped);
        }
        let region = Region {
            base: start,
            edge: end,
            flags,
            kind,
        };
        let idx = self.regions.partition_point(|r| r.base < start);
        self.regions.insert(idx, region);
        Ok(())
    }
}

// SAFETY: 全部可变状态由 `RelLock` 互斥（跨 hart 自旋）；页表树读写与 `SpaceInner`
// 共享同一把锁。`kind` / `root` 分配后不可变。
unsafe impl Send for Space {}
unsafe impl Sync for Space {}

/// [`Space`] 构造器 — 区分内核空间与用户空间。自身即为构建入口，
/// 不依赖 `Space` 签发。
///
/// ```ignore
/// let kernel = SpaceBuilder::kernel().build()?;
/// let user   = SpaceBuilder::user().build()?;
/// ```
pub struct SpaceBuilder {
    kind: SpaceKind,
}

impl SpaceBuilder {
    /// 内核空间构造器（ASID 0）。
    pub fn kernel() -> Self {
        Self {
            kind: SpaceKind::Kernel,
        }
    }

    /// 用户空间构造器（独立 ASID，经 [`super::asid::allocate`] 分配）。
    pub fn user() -> Self {
        Self {
            kind: SpaceKind::User {
                asid: super::asid::allocate(),
            },
        }
    }

    /// 完成构建：分配根页表帧；用户空间额外从内核空间种入 trampoline
    /// 叶 PTE 与 trap-context 帧（见 [`SpaceBuilder::seed_user`]）。
    ///
    /// # Errors
    ///
    /// 物理帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub fn build(self) -> Result<Space, MapError> {
        let mut space = Space {
            root: PageTable::root()?,
            kind: self.kind,
            inner: RelLock::new(SpaceInner::new()),
        };
        if matches!(space.kind, SpaceKind::User { .. }) {
            self.seed_user(&mut space)?;
        }
        Ok(space)
    }

    /// 从内核地址空间出用户空间（`build()` 内部调用）。
    ///
    /// 不复制内核半区映射——用户页表只含用户映射 + 两处固定 VA：
    /// - trampoline 叶 PTE 复制（[`Space::TRAMPOLINE`] VA → 内核 trampoline 物理页，**不拥有**）；
    /// - trap-context 帧（[`Space::TRAP_CONTEXT`] VA，**自有**，入 frames）。
    ///
    /// 内核切换元数据（kernel_satp / kernel_sp / trap_handler / trap_stack_corrupt）
    /// 从内核 trap-context 帧（trap::init 已写入）复制，`self_pa` 设为本帧物理地址。
    ///
    /// # Errors
    ///
    /// 页表/数据帧耗尽时返回 [`MapError::OutOfMemory`]。
    fn seed_user(&self, space: &mut Space) -> Result<(), MapError> {
        // 读内核空间的 trampoline 叶 PTE 与 trap-context 帧 PA（KERNEL_SPACE 只读）
        let (tramp_pa, tramp_flags, kernel_trap_context_pa) = {
            let guard = KERNEL_SPACE.lock();
            let ks = guard.as_ref().ok_or(MapError::NotMapped)?;
            let (tramp_pa, tramp_flags) = ks.root.as_ref().walk_ref(TRAMPOLINE)?;
            let (kpa, _) = ks.root.as_ref().walk_ref(TRAP_CONTEXT)?;
            (tramp_pa, tramp_flags, kpa)
        };

        // trampoline 叶（帧归内核，不入 frames）
        space.map(TRAMPOLINE, tramp_pa, crate::memory::PAGE_SIZE, tramp_flags)?;

        // trap-context 帧：分配 + 拷贝内核元数据 + 映射（自有，入 frames）
        let trap_context =
            Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
        let trap_context_pa = PhysAddr::from_raw(trap_context.as_ptr() as usize);
        // SAFETY: 内核帧 PA 恒等可读（trap::init 已写入元数据）；新帧独占。
        unsafe {
            let ktc =
                kernel_trap_context_pa.as_usize() as *const crate::runtime::context::TrapContext;
            let utc = trap_context_pa.as_usize() as *mut crate::runtime::context::TrapContext;
            (*utc).kernel_satp = (*ktc).kernel_satp;
            (*utc).kernel_sp = (*ktc).kernel_sp;
            (*utc).trap_handler = (*ktc).trap_handler;
            (*utc).trap_stack_corrupt = (*ktc).trap_stack_corrupt;
            (*utc).self_pa = trap_context_pa;
            // user_satp = Sv39 模式位(8) << 60 | asid << 44 | root_ppn —— __restore 切回本空间用
            (*utc).user_satp = (8usize << 60) | (space.asid() << 44) | space.root();
        }
        space.map(
            TRAP_CONTEXT,
            trap_context_pa,
            crate::memory::PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )?;
        space.inner.lock().frames.push(trap_context);
        Ok(())
    }
}

impl Space {
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

    // ── 映射操作 ──────────────────────────────────────────────

    /// 分配一个任务栈窗口，返回栈体 VA（16 KiB，向下增长，底部守护页）。
    ///
    /// 窗口 slot = 守护页 + 栈体，一次从 [`BitmapAllocator`]（`stacks` 实例）
    /// 领取；守护页 `[slot_va, slot_va + STACK_GUARD_SIZE)` 登记为 Reserved
    /// Region——栈溢出触碰守护页时 fault 处理器识别为「预留区域访问」而非
    /// 笼统的「无 Region」。栈体本身**不**映射、不登记：帧由 Task 侧自行分配
    /// （归 Task 所有权，不入本空间 `frames`）；如需懒分配可另行
    /// [`declare`](Self::declare)。窗口释放见 [`stack_dealloc`](Self::stack_dealloc)。
    #[allow(dead_code)] // 任务 spawn 后端预留
    pub(crate) fn stack_alloc(&self) -> Result<VirtAddr, MapError> {
        let mut inner = self.inner.lock();
        let slot_size = TASK_STACK_SIZE + STACK_GUARD_SIZE;
        let (slot_va, _) = inner
            .stacks
            .allocate(slot_size)
            .map_err(|_| MapError::OutOfMemory)?;
        let slot_va = VirtAddr::from_raw(slot_va);
        // 守护页 [slot_va, slot_va + STACK_GUARD_SIZE)：Reserved → 溢出缺页可诊断
        if let Err(e) = inner.declare(
            slot_va,
            STACK_GUARD_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W,
            RegionKind::Reserved,
        ) {
            let _ = inner.stacks.deallocate(slot_va.as_usize(), slot_size);
            return Err(e);
        }
        Ok(slot_va + STACK_GUARD_SIZE)
    }

    /// 释放任务栈窗口：清整窗口 PTE → 删守护页 Region → 窗口归还位图分配器。
    ///
    /// 栈帧所有权仍在 Task（unmap 只清 PTE 不碰帧）；**窗口复用安全的关键**：
    /// PTE 清理 + [`flush_asid`] 先于归还，杜绝新窗口摸到旧任务残留映射。
    #[allow(dead_code)] // 任务回收后端预留
    pub(crate) fn stack_dealloc(&self, stack_va: VirtAddr) -> bool {
        let slot_va = stack_va - STACK_GUARD_SIZE;
        let slot_size = TASK_STACK_SIZE + STACK_GUARD_SIZE;
        let mut inner = self.inner.lock();
        // 1. 清整窗口叶子 PTE（含 Task 侧栈帧映射；帧仍归 Task）
        for i in 0..slot_size / PAGE_SIZE {
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe { self.root_mut().unmap(slot_va + i * PAGE_SIZE) };
        }
        // 2. 删守护页 Region
        inner
            .regions
            .retain(|r| !(slot_va < r.edge && (slot_va + slot_size) > r.base));
        // 3. 窗口归还位图分配器（供后续 stack_alloc 复用）
        if inner
            .stacks
            .deallocate(slot_va.as_usize(), slot_size)
            .is_err()
        {
            return false;
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        true
    }

    /// 用户堆分配：经 [`BitmapAllocator::allocate`]（`heap` 实例）保留页对齐
    /// VA 块，逐页从 frame 分配器取物理页映射到用户区（U|R|W），并登记
    /// Anonymous Region（缺页/查询可识别堆区）。返回分配 VA。
    ///
    /// 堆窗口固定 [`USER_HEAP_BASE`] 起 [`USER_HEAP_SIZE`]；窗口耗尽 →
    /// [`MapError::OutOfMemory`]。立即分配（非懒分配）：教学简化，页表与
    /// 物理页当场就位，用户访问不再缺页。中途帧耗尽时回滚：清掉已映射页的
    /// 叶子并归还其数据帧，VA 块退回复用（[`BitmapAllocator::deallocate`]，
    /// 中间表帧仍留 `frames`，随 Space Drop 归还）。
    #[allow(dead_code)] // map/unmap syscall 后端预留
    pub(crate) fn heap_allocate(&self, size: usize) -> Result<VirtAddr, MapError> {
        let mut inner = self.inner.lock();
        // 1. 堆 VA 簿记：位图分配器保留块（first-fit，释放后可复用）
        let (base, size) = inner
            .heap
            .allocate(size)
            .map_err(|_| MapError::OutOfMemory)?;
        let base = VirtAddr::from_raw(base);
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        // 2. 登记 Anonymous Region（resolve/缺页可识别堆区；与 unmap/释放配对）
        if let Err(e) = inner.declare(base, size, flags, RegionKind::Anonymous) {
            let _ = inner.heap.deallocate(base.as_usize(), size);
            return Err(e);
        }
        // 3. 逐页分配物理帧并映射（立即分配）
        let pages = size / crate::memory::PAGE_SIZE;
        let mut mapped = 0usize;
        while mapped < pages {
            let page = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            inner.frames.push(page);
            let va = base + mapped * crate::memory::PAGE_SIZE;
            // SAFETY: 持 inner 锁期间修改页表。
            if unsafe {
                self.root_mut()
                    .map(va, pa, crate::memory::PAGE_SIZE, flags, &mut inner.frames)
            }
            .is_err()
            {
                // 回滚：清掉本块已映射页的叶子并归还其数据帧；VA 块退回位图
                for i in 0..mapped {
                    let v = base + i * crate::memory::PAGE_SIZE;
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
                    .retain(|r| !(base < r.edge && (base + size) > r.base));
                let _ = inner.heap.deallocate(base.as_usize(), size);
                drop(inner);
                // SAFETY: S-mode 下 sfence.vma 恒合法。
                unsafe {
                    flush_asid(self.kind.asid());
                }
                return Err(MapError::OutOfMemory);
            }
            mapped += 1;
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(base)
    }

    /// 用户堆释放：位图精确匹配 `(addr, size)` 后 unmap 并归还物理页。
    ///
    /// 持锁一次性完成：[`BitmapAllocator::deallocate`]（`heap` 实例）删块 →
    /// 逐页 translate 取物理帧 → retain 从 frames 移除（Box Drop 归还 frame
    /// 池）→ 清叶子 PTE → 删 Region → 刷 TLB。返回是否找到并释放（未分配/
    /// 部分已释放的区间返回 false，同旧块表精确匹配语义）。
    #[allow(dead_code)] // map/unmap syscall 后端预留
    pub(crate) fn heap_deallocate(&self, addr: VirtAddr, size: usize) -> bool {
        let mut inner = self.inner.lock();
        // 1. 位图精确匹配释放（未分配/部分已释放 → 返回 false）
        if inner.heap.deallocate(addr.as_usize(), size).is_err() {
            return false;
        }
        // 2. 逐页归还物理帧并清叶子
        for i in 0..size / crate::memory::PAGE_SIZE {
            let v = addr + i * crate::memory::PAGE_SIZE;
            if let Some((pa, _)) = self.translate_inner(v) {
                // retain 丢弃的 Frame 由 Drop 归还 frame 池
                inner
                    .frames
                    .retain(|f| f.as_ptr() as usize != pa.as_usize());
            }
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe { self.root_mut().unmap(v) };
        }
        // 3. 删对应 Region
        inner
            .regions
            .retain(|r| !(addr < r.edge && (addr + size) > r.base));
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
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
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 取消映射一段虚拟地址并移除其 Region 记录（ecall munmap 后端）。
    ///
    /// 页表侧逐页清叶子 PTE（惰性策略，不释放中间页表）；Region 侧按重叠
    /// 删除与 `[start, start+size)` 相交的所有记录。`vaddr`/`size` 不要求
    /// 页对齐（向上取整语义与 POSIX munmap 一致）。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        let end = vaddr + size;

        let mut inner = self.inner.lock();
        // 页表侧：逐页清叶子 PTE
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe { self.root_mut().unmap(vaddr + i * PAGE_SIZE) };
        }
        // Region 侧：删重叠记录
        inner.regions.retain(|r| !(vaddr < r.edge && end > r.base));
        drop(inner);

        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
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
            flush_asid(self.kind.asid());
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
            let page = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            inner.frames.push(page);
            // SAFETY: 持 inner 锁期间修改页表。
            unsafe {
                self.root_mut()
                    .map(va, pa, PAGE_SIZE, flags, &mut inner.frames)?;
            }
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
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
        self.kind.asid()
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
        start: VirtAddr,
        size: usize,
        flags: PteFlags,
        kind: RegionKind,
    ) -> Result<(), MapError> {
        self.inner.lock().declare(start, size, flags, kind)
    }

    /// 查询虚拟地址所属的 Region。
    ///
    /// 返回 `Option<Region>`（Copy）而非引用：Region 表在锁内，
    /// borrow 不能跨锁返回引用。
    pub fn resolve(&self, vaddr: VirtAddr) -> Option<Region> {
        self.inner.lock().resolve(vaddr)
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        // 先释放本空间的 ASID：`free` 内部会 sfence 该 ASID 的 TLB 残留条目
        // （ASID 可能被后续任务复用，旧条目须失效）。内核空间 ASID 0 不参与分配。
        if let SpaceKind::User { asid } = self.kind {
            super::asid::deallocate(asid);
        }
        // `root`（Box<PageTable>）与 `frames`（Vec<Frame>）随字段自动 drop 归还
        // frame 池——所有权驱动，无遍历页表树、无手写 deallocate。
    }
}

impl Space {
    /// 登记一个本空间拥有的帧（中间表 / 数据帧 / trap-context 帧）——供内核
    /// 空间 init 等把已分配的帧纳入所有权，Box Drop 时自动归还 frame 池。
    pub(crate) fn track_frame(&self, frame: Box<[u8; PAGE_SIZE], &'static dyn Allocator>) {
        self.inner.lock().frames.push(frame);
    }
}

// ── 内核地址空间 ─────────────────────────────────────────────

/// 内核地址空间。`memory::init()` 创建并写入，此后只读访问。
///
/// 用 RelLock（可重入锁）：持有此锁期间若触发缺页，缺页处理器（trap.rs）
/// 会在同一 hart 上再次获取它——RelLock 允许同 hart 重入，避免自旋死锁；
/// 不同 hart 之间仍互斥。
pub(crate) static KERNEL_SPACE: RelLock<Option<Space>> = RelLock::new(None);

/// 获取内核地址空间的锁保护引用。
pub(crate) fn kernel_space() -> RelLockGuard<'static, Option<Space>> {
    KERNEL_SPACE.lock()
}

/// 内核 trap-context 帧物理地址（`init()` 写入；trap::init 写元数据、idle_frame
/// 写空闲合成帧、用户空间构建从它拷内核切换信息）。
pub static KERNEL_TRAP_CONTEXT: AtomicPhysAddr = AtomicPhysAddr::new(PhysAddr::from_raw(0));

/// 内核 trap-context 帧物理地址。
pub fn kernel_trap_context() -> PhysAddr {
    KERNEL_TRAP_CONTEXT.load(Ordering::Relaxed)
}
