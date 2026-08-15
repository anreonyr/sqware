// 地址空间 — MMU 子系统的核心抽象
//
// Space 拥有一个 Sv39 根页表与全部自有物理帧，提供虚拟→物理映射、权限管理、
// 地址翻译等高层操作。空间种类由 [`SpaceKind`] 显式区分：内核空间（ASID 0，
// 全局唯一）与用户空间（独立 ASID），构造统一走 [`SpaceBuilder`]。
//
// 簿记模型（三层，语义 = VA→PA 的显式表达）：
//   Durable — 常数侧：根页表（root）+ 中间表帧（tables）+ 常数映射表（maps：
//             DRAM 恒等/内核高半区、trampoline 叶 PTE、文本、trap-context 帧）
//   Window  — 动态侧：堆/栈窗口，各持一个 [`BitmapAllocator`] 细分窗口内 VA，
//             每次分配的一块区间即一个子 Map（children）——heap 块 / 栈 slot
//   Map     — VA→PA 的原子单元：{ va, size, flags, kind, frames }，不变量
//             frames[i] ↔ va + i·PAGE_SIZE；帧随 Map 所有权回收（drop 归还）
//
// 路线 1 后用户空间不共享内核映射——trampoline 叶 PTE 只映射不拥有（帧归内核，
// Map.frames 为空）；其余帧（文本、trap-context、堆块、栈体、中间表）全归本
// 空间所有。中间表帧无用户 VA 身份，收进 Durable::tables 平铺持有（树状所有权
// 会使 PageTable 超过 4096 B，见 table.rs 的尺寸约束）。
//
// 锁（`Space::inner`，RelLock）：全部可变状态同锁互斥；ASID 空间为位图分配器
// 的全局实例（见 `memory::manager::asid`）。

use alloc::vec;
use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box};
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;

use crate::lock::reentrant::RelLockGuard;
use crate::memory::allocator::bitmap::BitmapAllocator;
use crate::memory::allocator::frame::allocator;
use crate::{lock::RelLock, memory::PAGE_SIZE};

use super::{
    addr::{AtomicPhysAddr, PhysAddr, VirtAddr},
    entry::PteFlags,
    flush_asid,
    table::{Frame, MapError, PageTable},
};

/// 映射种类 — 缺页时如何响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapKind {
    /// 匿名映射 — 缺页时分配零页
    Anonymous,
    /// 预留映射 — 不可访问，缺页时返回错误
    ///
    /// 任务栈守护页与内核借用映射（DRAM 恒等、trampoline 叶）以 Reserved 登记：
    /// 越权触碰时 fault.rs 返回「预留映射访问」而非笼统的「无 Map」。
    Reserved,
}

/// 映射摘要 — [`Map`] 的 Copy 视图（锁外查询用；Map 含 Vec 不可 Copy）。
/// 不实现 `PartialEq`：`PteFlags`（bitflags）未提供该 trait。
#[derive(Debug, Clone, Copy)]
pub struct MapInfo {
    /// 映射起始虚拟地址。
    pub(crate) va: VirtAddr,
    /// 映射大小（字节，非零）。
    pub(crate) size: NonZeroUsize,
    /// 映射权限标志。
    pub(crate) flags: PteFlags,
    /// 映射种类。
    pub(crate) kind: MapKind,
}

/// 虚拟→物理映射 — 簿记的原子单元。
///
/// 语义：本映射覆盖 VA 区间 `[va, va + size)`；`frames[i]` 是 VA
/// `va + i·PAGE_SIZE` 处物理帧的持有者（**不变量**：帧 i ↔ va + i·PAGE_SIZE）。
/// 借用映射（DRAM 恒等、trampoline 叶：物理帧归内核/机器）frames 为空，PA 由
/// 页表维护；拥有映射（文本、trap-context、堆块、栈体）帧随 Map drop 归还 frame
/// 池——所有权即回收，无遍历页表树、无手写 deallocate。
#[derive(Debug)]
pub struct Map {
    va: VirtAddr,
    size: NonZeroUsize,
    flags: PteFlags,
    kind: MapKind,
    frames: Vec<Frame>,
}

impl Map {
    /// 构造（size 必须非零——调用方保证，见各入口的校验）。
    fn new(va: VirtAddr, size: usize, flags: PteFlags, kind: MapKind, frames: Vec<Frame>) -> Self {
        Self {
            va,
            size: NonZeroUsize::new(size).expect("map size must be non-zero"),
            flags,
            kind,
            frames,
        }
    }

    /// Copy 摘要。
    fn info(&self) -> MapInfo {
        MapInfo {
            va: self.va,
            size: self.size,
            flags: self.flags,
            kind: self.kind,
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 偏移 `off`（页内偏移无关，按页取帧）处的物理地址。
    ///
    /// # Errors
    ///
    /// 越界（`off >= size`）或该页尚无帧（懒分配未就位）→ [`MapError::NotMapped`]。
    #[allow(dead_code)] // munmap/mprotect 后端预留：按 VA→PA 语义直接取帧 PA
    fn translate(&self, off: usize) -> Result<PhysAddr, MapError> {
        if off >= self.size.get() {
            return Err(MapError::NotMapped);
        }
        let idx = off / PAGE_SIZE;
        let frame = self.frames.get(idx).ok_or(MapError::NotMapped)?;
        Ok(PhysAddr::from_raw(frame.as_ptr() as usize))
    }

    /// 追加一帧（保持不变量；调用方保证序号连续且不越界）。
    fn push_frame(&mut self, frame: Frame) {
        debug_assert!(
            self.frames.len() < self.size.get() / PAGE_SIZE,
            "map {:#x} frame overflow",
            self.va.as_usize()
        );
        self.frames.push(frame);
    }
}

/// 窗口种类 — 动态区域的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// 用户堆窗口（[`USER_HEAP_BASE`] 起 [`USER_HEAP_SIZE`]）。
    Heap,
    /// 任务栈窗口（[`USER_STACK_BASE`] 起 [`TASK_STACK_AREA_SIZE`]，约 5 万 slot）。
    Stack,
}

/// 动态窗口 — 固定 VA 区间 + 位图分配器 + 结构性绑定的子映射表。
///
/// `children` 记录本窗口已分配出去的每块区间（heap 块 / 栈 slot 的守护页与栈体），
/// 与位图分配器的存活块一一对应——释放即从 children 移除（帧随 Map drop 归还），
/// 窗口可安全复用（PTE 已清、无残留映射）。
#[derive(Debug)]
pub struct Window {
    kind: WindowKind,
    va: VirtAddr,
    size: NonZeroUsize,
    alloc: BitmapAllocator,
    children: Vec<Map>,
}

impl Window {
    fn new(kind: WindowKind) -> Self {
        let (base, size) = match kind {
            WindowKind::Heap => (USER_HEAP_BASE.as_usize(), USER_HEAP_SIZE),
            WindowKind::Stack => (USER_STACK_BASE.as_usize(), TASK_STACK_AREA_SIZE),
        };
        Self {
            kind,
            va: VirtAddr::from_raw(base),
            size: NonZeroUsize::new(size).expect("window size non-zero"),
            alloc: BitmapAllocator::new(base, base + size, PAGE_SIZE),
            children: Vec::new(),
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 从位图分配一块 VA，登记一个空帧子 Map（懒分配：帧由调用方随后注入）。
    fn allocate(
        &mut self,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
    ) -> Result<MapInfo, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let (base, size) = self.alloc.allocate(size).map_err(|_| MapError::OutOfMemory)?;
        let map = Map::new(VirtAddr::from_raw(base), size, flags, kind, Vec::new());
        let info = map.info();
        self.children.push(map);
        Ok(info)
    }

    /// 释放一块 VA：位图精确匹配成功后移除重叠子 Map（帧随 drop 归还）。
    fn deallocate(&mut self, va: VirtAddr, size: usize) -> bool {
        if self.alloc.deallocate(va.as_usize(), size).is_err() {
            return false;
        }
        let end = va.as_usize().saturating_add(size);
        self.children.retain(|m| {
            !(va.as_usize() < m.va.as_usize().saturating_add(m.size.get())
                && end > m.va.as_usize())
        });
        true
    }
}

/// 常数侧 — 根页表 + 中间表帧 + 常数映射表。
///
/// `maps` 覆盖空间建立期就确定、生命周期与空间一致的映射（内核 DRAM 恒等/
/// 高半区、trampoline 叶、文本、trap-context 帧）；`tables` 收集全部中间页表
/// 帧（无用户 VA 身份，不占 maps 槽位）。Drop 统一归还。
#[derive(Debug)]
pub struct Durable {
    root: Box<PageTable, &'static dyn Allocator>,
    tables: Vec<Frame>,
    maps: Vec<Map>,
}

impl Durable {
    fn new() -> Result<Self, MapError> {
        Ok(Self {
            root: PageTable::root()?,
            tables: Vec::new(),
            maps: Vec::new(),
        })
    }

    /// 查询覆盖 `vaddr` 的常数映射。
    fn resolve(&self, vaddr: VirtAddr) -> Option<&Map> {
        self.maps.iter().rev().find(|m| m.contains(vaddr))
    }

    /// 查询覆盖 `vaddr` 的常数映射（可变，缺页注入帧用）。
    fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        self.maps.iter_mut().rev().find(|m| m.contains(vaddr))
    }

    /// `[start, start+size)` 是否与已有常数映射或窗口区间重叠。
    ///
    /// 边界用 saturating 算术：TRAMPOLINE 等最高页映射的 `va + size` 会溢出 2^64，
    /// 饱和到 `usize::MAX` 即「延伸到地址空间尽头」——比较仍正确。
    fn overlaps(&self, start: VirtAddr, size: usize, windows: &[Window]) -> bool {
        let end = start.as_usize().saturating_add(size);
        self.maps.iter().any(|m| {
            start.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize()
        }) || windows.iter().any(|w| {
            start.as_usize() < w.va.as_usize().saturating_add(w.size.get()) && end > w.va.as_usize()
        })
    }
}

/// 地址空间的可变状态 — 由 [`Space::inner`] 这把 [`RelLock`] 保护。
///
/// 只做组合：`durable`（常数侧）与 `dynamic`（堆/栈窗口）覆盖空间全部簿记；
/// 与页表操作同锁互斥（锁约定见 [`Space`]）。
#[derive(Debug)]
struct SpaceInner {
    /// 常数侧：根页表 + 中间表帧 + 常数映射表。
    durable: Durable,
    /// 动态侧：堆 / 栈窗口（各自位图分配器 + 子 Map 表）。
    dynamic: Vec<Window>,
}

impl SpaceInner {
    fn new() -> Result<Self, MapError> {
        Ok(Self {
            durable: Durable::new()?,
            dynamic: vec![Window::new(WindowKind::Heap), Window::new(WindowKind::Stack)],
        })
    }


    /// 查询 `vaddr` 所属的映射（常数表 → 动态窗口子表），返回 Copy 摘要。
    fn resolve(&self, vaddr: VirtAddr) -> Option<MapInfo> {
        if let Some(m) = self.durable.resolve(vaddr) {
            return Some(m.info());
        }
        for w in &self.dynamic {
            if w.contains(vaddr) {
                if let Some(m) = w.children.iter().rev().find(|m| m.contains(vaddr)) {
                    return Some(m.info());
                }
            }
        }
        None
    }

    /// 查询 `vaddr` 所属映射的可变引用（缺页注入帧用）。
    fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        if let Some(m) = self.durable.resolve_mut(vaddr) {
            return Some(m);
        }
        for w in &mut self.dynamic {
            if w.contains(vaddr) {
                if let Some(m) = w.children.iter_mut().rev().find(|m| m.contains(vaddr)) {
                    return Some(m);
                }
            }
        }
        None
    }

    /// 登记常数映射（内部版，调用者须持锁；不装 PTE、不注入帧——懒区域用）。
    #[allow(dead_code)] // 经 pub Space::declare 暴露，懒区域（ecall mmap 预留）后端接入前无调用方
    fn declare_durable(
        &mut self,
        start: VirtAddr,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
    ) -> Result<(), MapError> {
        if size == 0
            || !start.as_usize().is_multiple_of(PAGE_SIZE)
            || !size.is_multiple_of(PAGE_SIZE)
        {
            return Err(MapError::NotAligned);
        }
        if self.durable.overlaps(start, size, &self.dynamic) {
            return Err(MapError::AlreadyMapped);
        }
        self.durable
            .maps
            .push(Map::new(start, size, flags, kind, Vec::new()));
        Ok(())
    }

    /// 页表读翻译（内部版，调用者须持锁，与 map/unmap 写互斥）。
    fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.durable.root.as_ref().walk_ref(vaddr).ok()
    }
}

/// 按种类取窗口的可变引用（调用者须持锁）。
///
/// 只借 `windows` 切片本身（`&mut [Window]`）——与 `durable` 字段的借用共存：
/// 同时需要两者时先 `let SpaceInner { durable, dynamic } = &mut *guard` 解构。
fn window_mut(windows: &mut [Window], kind: WindowKind) -> &mut Window {
    windows
        .iter_mut()
        .find(|w| w.kind == kind)
        .expect("window exists")
}

// SAFETY: 全部可变状态由 `RelLock` 互斥（跨 hart 自旋）；页表树读写与 `SpaceInner`
// 共享同一把锁。`kind` 分配后不可变。
unsafe impl Send for Space {}
unsafe impl Sync for Space {}

/// 空间种类 — 显式区分内核空间与用户空间。
///
/// 内核空间 ASID 恒 0、全局唯一；用户空间各自持有独立 ASID（1..=65535），
/// 构造时经 [`super::asid::allocate`] 分配、`Drop` 释放。
///
/// 用户区布局常量（堆窗口 / 任务栈窗口）收敛进本模块（`USER_HEAP_*` / `TASK_STACK_*`），
/// 由 [`Window`] 的位图分配器实例消费。
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

// ── 地址空间布局常量 ────────────────────────────────────────

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
/// 帧——自有）。全部可变状态（`SpaceInner`：durable / dynamic）收进一把
/// [`RelLock`]；根页表随 `durable.root` 持有（Box 自动 drop）。
///
/// 空间种类由 [`kind`](Self::kind) 显式区分（见 [`SpaceKind`]）。
///
/// # Concurrency
///
/// 全部可变状态（`SpaceInner`）收进一把 [`RelLock`]：跨 hart 真自旋互斥、
/// 同 hart 可重入——多核下两个 hart 共享同一空间做 map/unmap/缺页时互斥；同
/// hart 持锁期间同步缺页（异步中断不受 SIE 屏蔽）可重入。**约定**：每个公开
/// 方法锁恰好一次，内部直接操作 `inner`（经 `SpaceInner` 方法），不重入——
/// 重入时若两个 guard 同时 DerefMut 会构成 `&mut` 别名（UB）。
///
/// 页表树读写与 `SpaceInner` 数据共享同一把锁：`translate` 读页表、`map`/
/// `unmap` 写页表，都要持锁互斥（页表修改跨核可见性由锁的 Release/Acquire 保证）。
///
/// 借用约定：guard 是 `Deref`，方法调用的自动引用会借整个 deref 目标——
/// 需要同时借 `durable` 的不同字段（如 `root` 与 `tables`）时，先绑定局部
/// 变量（字段级拆借），再调用方法。
///
/// # Drop
///
/// `durable.root`（Box）、`durable.tables`、`durable.maps` 与 `dynamic` 窗口的
/// 子 Map 帧随字段自动 drop 归还 frame 池——所有权驱动，无需遍历页表树、
/// 无需手写 deallocate。
#[derive(Debug)]
pub struct Space {
    /// 全部可变状态（durable / dynamic）——一把可重入锁保护。
    inner: RelLock<SpaceInner>,
    /// 空间种类（内核 / 用户），内嵌 ASID。
    kind: SpaceKind,
}

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
            kind: self.kind,
            inner: RelLock::new(SpaceInner::new()?),
        };
        if matches!(space.kind, SpaceKind::User { .. }) {
            self.seed_user(&mut space)?;
        }
        Ok(space)
    }

    /// 从内核地址空间出用户空间（`build()` 内部调用）。
    ///
    /// 不复制内核半区映射——用户页表只含用户映射 + 两处固定 VA：
    /// - trampoline 叶 PTE 复制（[`Space::TRAMPOLINE`] VA → 内核 trampoline 物理页，**不拥有**），
    ///   以 Reserved 常数映射登记（借用：无帧）；
    /// - trap-context 帧（[`Space::TRAP_CONTEXT`] VA，**自有**，帧入常数映射）。
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
            let ks_inner = ks.inner.lock();
            let (tramp_pa, tramp_flags) = ks_inner.durable.root.as_ref().walk_ref(TRAMPOLINE)?;
            let (kpa, _) = ks_inner.durable.root.as_ref().walk_ref(TRAP_CONTEXT)?;
            (tramp_pa, tramp_flags, kpa)
        };

        // trampoline 叶（帧归内核，借用：Reserved 常数映射、无帧）
        space.map(TRAMPOLINE, tramp_pa, PAGE_SIZE, tramp_flags, MapKind::Reserved, Vec::new())?;

        // trap-context 帧：分配 + 拷贝内核元数据 + 映射（自有，帧入常数映射）
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
            (*utc).user_pa = trap_context_pa;
            // user_satp = Sv39 模式位(8) << 60 | asid << 44 | root_ppn —— __restore 切回本空间用
            (*utc).user_satp = (8usize << 60) | (space.asid() << 44) | space.root();
        }
        space.map(
            TRAP_CONTEXT,
            trap_context_pa,
            PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
            MapKind::Anonymous,
            vec![trap_context],
        )?;
        Ok(())
    }
}

impl Space {
    // ── 映射操作 ──────────────────────────────────────────────

    /// 分配一个任务栈窗口 slot，返回栈体 VA（16 KiB，向下增长，底部守护页）。
    ///
    /// slot = 守护页 + 栈体，一次从 Stack 窗口位图领取；守护页登记为 Reserved
    /// 子 Map——栈溢出触碰守护页时 fault 处理器识别为「预留映射访问」而非笼统的
    /// 「无 Map」。栈体登记为 Anonymous 子 Map（帧随后经
    /// [`stack_attach`](Self::stack_attach) 注入）。窗口释放见
    /// [`stack_dealloc`](Self::stack_dealloc)。
    pub(crate) fn stack_alloc(&self) -> Result<VirtAddr, MapError> {
        let mut inner = self.inner.lock();
        let slot_size = TASK_STACK_SIZE + STACK_GUARD_SIZE;
        let stack = window_mut(&mut inner.dynamic, WindowKind::Stack);
        let (slot_va, _) = stack
            .alloc
            .allocate(slot_size)
            .map_err(|_| MapError::OutOfMemory)?;
        let slot_va = VirtAddr::from_raw(slot_va);
        // 守护页 [slot_va, slot_va + STACK_GUARD_SIZE)：Reserved → 溢出缺页可诊断
        stack.children.push(Map::new(
            slot_va,
            STACK_GUARD_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W,
            MapKind::Reserved,
            Vec::new(),
        ));
        // 栈体 [slot_va + GUARD, +TASK_STACK_SIZE)：Anonymous，帧待 attach
        let body_va = slot_va + STACK_GUARD_SIZE;
        stack.children.push(Map::new(
            body_va,
            TASK_STACK_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D,
            MapKind::Anonymous,
            Vec::new(),
        ));
        Ok(body_va)
    }

    /// 把栈体帧注入并映射（spawn 用：stack_alloc 取 VA 后分配帧再 attach）。
    ///
    /// 逐帧：PTE 安装（新中间表帧入 `durable.tables`）+ `push_frame`（入栈体子
    /// Map，保持「帧 i ↔ va + i·PAGE_SIZE」不变量）。中途失败返回错误，调用方
    /// drop Space 统一回收（无部分状态残留担忧——已装 PTE 与已入帧随空间归还）。
    pub(crate) fn stack_attach(
        &self,
        stack_va: VirtAddr,
        frames: Vec<Frame>,
    ) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        let stack = window_mut(dynamic, WindowKind::Stack);
        let child = stack
            .children
            .iter_mut()
            .find(|m| m.va == stack_va)
            .ok_or(MapError::NoRegion)?;
        let child_flags = child.flags;
        for (i, frame) in frames.into_iter().enumerate() {
            let pa = PhysAddr::from_raw(frame.as_ptr() as usize);
            let va = stack_va + i * PAGE_SIZE;
            // 解构后 durable 内部字段（root/tables）可直接拆借调用
            durable.root.map(va, pa, PAGE_SIZE, child_flags, &mut durable.tables)?;
            child.push_frame(frame);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 释放任务栈窗口：清整窗口 PTE → 移除子 Map（帧随 drop 归还）→ 窗口归还位图。
    ///
    /// **窗口复用安全的关键**：PTE 清理 + [`flush_asid`] 先于归还，杜绝新窗口
    /// 摸到旧任务残留映射。
    #[allow(dead_code)] // 任务回收后端预留
    pub(crate) fn stack_dealloc(&self, stack_va: VirtAddr) -> bool {
        let slot_va = stack_va - STACK_GUARD_SIZE;
        let slot_size = TASK_STACK_SIZE + STACK_GUARD_SIZE;
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        // 1. 清整窗口叶子 PTE（含栈体帧映射；帧归子 Map，随后 drop 归还）
        for i in 0..slot_size / PAGE_SIZE {
            durable.root.unmap(slot_va + i * PAGE_SIZE);
        }
        // 2. 窗口释放：位图精确匹配 + 移除守护页/栈体子 Map
        let stack = window_mut(dynamic, WindowKind::Stack);
        if !stack.deallocate(slot_va, slot_size) {
            return false;
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        true
    }

    /// 用户堆分配：经 Heap 窗口位图保留页对齐 VA 块，登记空子 Map（Anonymous），
    /// 逐页从 frame 分配器取物理页映射到用户区（U|R|W）并注入子 Map。返回分配 VA。
    ///
    /// 堆窗口固定 [`USER_HEAP_BASE`] 起 [`USER_HEAP_SIZE`]；窗口耗尽 →
    /// [`MapError::OutOfMemory`]。立即分配（非懒分配）：教学简化，页表与物理页
    /// 当场就位，用户访问不再缺页。中途帧耗尽时回滚：清已映射页叶子 + 移除子
    /// Map（帧随 drop 归还）+ VA 块退回复用（[`BitmapAllocator::deallocate`]；
    /// 中间表帧仍留 `durable.tables`，随 Space Drop 归还）。
    #[allow(dead_code)] // map/unmap syscall 后端预留
    pub(crate) fn heap_allocate(&self, size: usize) -> Result<VirtAddr, MapError> {
        let mut inner = self.inner.lock();
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        let SpaceInner { durable, dynamic } = &mut *inner;
        // 1. 窗口位图保留块 + 登记空子 Map（Anonymous）
        let heap = window_mut(dynamic, WindowKind::Heap);
        let info = heap.allocate(size, flags, MapKind::Anonymous)?;
        // 2. 逐页分配物理帧并映射（立即分配）+ 注入子 Map
        let pages = info.size.get() / PAGE_SIZE;
        let mut mapped = 0usize;
        while mapped < pages {
            let page = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            let va = info.va + mapped * PAGE_SIZE;
            if durable.root.map(va, pa, PAGE_SIZE, flags, &mut durable.tables).is_err() {
                // 回滚：清已映射页叶子 + 移除子 Map（帧随 drop 归还）+ VA 退回位图
                for i in 0..mapped {
                    durable.root.unmap(info.va + i * PAGE_SIZE);
                }
                heap.deallocate(info.va, info.size.get());
                drop(inner);
                // SAFETY: S-mode 下 sfence.vma 恒合法。
                unsafe {
                    flush_asid(self.kind.asid());
                }
                return Err(MapError::OutOfMemory);
            }
            let child = heap
                .children
                .iter_mut()
                .find(|m| m.va == info.va)
                .expect("heap child exists");
            child.push_frame(page);
            mapped += 1;
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(info.va)
    }

    /// 用户堆释放：位图精确匹配 `(addr, size)` 后清叶子 PTE 并移除子 Map（帧随
    /// drop 归还 frame 池）。返回是否找到并释放（未分配/部分已释放的区间返回
    /// false，同旧块表精确匹配语义）。
    #[allow(dead_code)] // map/unmap syscall 后端预留
    pub(crate) fn heap_deallocate(&self, addr: VirtAddr, size: usize) -> bool {
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        let heap = window_mut(dynamic, WindowKind::Heap);
        // 1. 位图精确匹配释放 + 移除子 Map（未分配/部分已释放 → 返回 false）
        if !heap.deallocate(addr, size) {
            return false;
        }
        // 2. 逐页清叶子 PTE（帧已随子 Map 移除 drop 归还）
        for i in 0..size / PAGE_SIZE {
            durable.root.unmap(addr + i * PAGE_SIZE);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        true
    }

    /// 映射 `size` 字节虚拟地址到物理地址（常数映射唯一公共入口）。
    ///
    /// PTE 安装 + 登记常数 [`Map`]（VA→PA 簿记）一次完成：`kind` 决定缺页响应，
    /// `frames` 为本映射**拥有**的帧（借用映射——DRAM 恒等、trampoline 叶——传空）。
    /// 按需分配中间页表——新帧收集进 [`durable.tables`](Durable::tables)。
    ///
    /// **vaddr、paddr、size 必须全部按 [`PAGE_SIZE`] 对齐**，且不得与已有映射/
    /// 窗口区间重叠。非对齐大小的调用方（如 MMIO 设备映射）须自行向上取整。
    ///
    /// 路线 1 后无共享子树：本空间所有页表帧私有，`map` 直接写叶子，无需 COW。
    ///
    /// # Errors
    ///
    /// 参见 [`PageTable::map`] 与 [`MapError::AlreadyMapped`]。
    pub fn map(
        &self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
        frames: Vec<Frame>,
    ) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        // 0. 参数校验（先于任何页表修改）
        if size == 0 || vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0
        {
            return Err(MapError::NotAligned);
        }
        // 1. 重叠检查（常数表 + 窗口区间）
        if inner.durable.overlaps(vaddr, size, &inner.dynamic) {
            return Err(MapError::AlreadyMapped);
        }
        // 2. PTE 安装 + 3. 登记常数映射（一次解构拿 durable 各字段，字段间可拆借）
        let SpaceInner { durable, .. } = &mut *inner;
        durable.root.map(vaddr, paddr, size, flags, &mut durable.tables)?;
        durable.maps.push(Map::new(vaddr, size, flags, kind, frames));
        drop(inner);
        // 按本空间 ASID 局部刷：只失效本地址空间的旧条目，其它任务 TLB 保留。
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 取消映射一段虚拟地址并移除其簿记（ecall munmap 后端）。
    ///
    /// 页表侧逐页清叶子 PTE（惰性策略，不释放中间页表）；簿记侧移除被 `[start,
    /// start+size)` **完全覆盖**的映射（帧随 drop 归还——unmap 即释放）。部分
    /// 重叠的映射保留记录、仅清 PTE：munmap 边界拆分留待后端接入时实现。
    /// `vaddr`/`size` 不要求页对齐（向上取整语义与 POSIX munmap 一致）。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        let end = vaddr.as_usize().saturating_add(size);

        let mut inner = self.inner.lock();
        // 页表侧：逐页清叶子 PTE
        let root = &mut inner.durable.root;
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            root.unmap(vaddr + i * PAGE_SIZE);
        }
        // 簿记侧：移除被完全覆盖的常数映射与窗口子 Map
        let covered = |m: &Map| {
            vaddr.as_usize() <= m.va.as_usize()
                && end >= m.va.as_usize().saturating_add(m.size.get())
        };
        inner.durable.maps.retain(|m| !covered(m));
        for w in &mut inner.dynamic {
            w.children.retain(|m| !covered(m));
        }
        drop(inner);

        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
    }

    /// 修改已映射区域的保护标志。
    ///
    /// 单次遍历，不分配中间表——叶子 PTE 不存在则返回错误。簿记侧 Map.flags
    /// 不同步（mprotect 后端接入时一并处理）。
    ///
    /// # Errors
    ///
    /// 任一页的叶子 PTE 不存在时返回 [`MapError::NotMapped`]。
    #[allow(dead_code)] // ecall mprotect 后端预留
    pub fn protect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let root = &mut inner.durable.root;
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            // None = 不分配中间表：叶子缺失即 NotMapped
            let leaf = root.walk_mut(va, None)?;
            leaf.set_flags(flags | PteFlags::V);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    // ── 缺页处理 ──────────────────────────────────────────────

    /// 缺页处理：查 Anonymous 映射 → 分配零页 → 映射 + 注入帧。
    ///
    /// 从 frame 分配器逐页取物理帧，清零后映射到 `vaddr` 起始的连续区间。
    /// 目标地址必须已登记 Anonymous 映射（懒分配按序补帧：自末帧起逐页填，
    /// 乱序/重复由 Map 的 debug_assert 暴露）。
    ///
    /// # Errors
    ///
    /// - [`MapError::NoRegion`] — 地址不在任何 Anonymous 映射内
    /// - [`MapError::OutOfMemory`] — 物理帧耗尽
    pub fn page_fault(
        &self,
        vaddr: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            // 前提：地址已登记 Anonymous 映射
            let info = inner.resolve(va).ok_or(MapError::NoRegion)?;
            if info.kind != MapKind::Anonymous {
                return Err(MapError::NoRegion);
            }
            let page = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            {
                let SpaceInner { durable, .. } = &mut *inner;
                durable.root.map(va, pa, PAGE_SIZE, flags, &mut durable.tables)?;
            }
            let map = inner.resolve_mut(va).expect("map exists (checked above)");
            map.push_frame(page);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    // ── 查询 ──────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位（页表读路径）。
    ///
    /// 未映射时返回 `None`。持锁与 map/unmap 的页表写互斥。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        let inner = self.inner.lock();
        inner.translate(vaddr)
    }

    /// 返回根页表页号（写入 `satp` 用）。
    pub fn root(&self) -> usize {
        let inner = self.inner.lock();
        Box::as_ptr(&inner.durable.root) as usize >> crate::memory::PAGE_SHIFT
    }

    /// 返回本空间的 ASID（写入 `satp.ASID` 用；0 = 内核空间）。
    pub fn asid(&self) -> usize {
        self.kind.asid()
    }

    // ── Map 管理 ──────────────────────────────────────────────

    /// 声明一段预留虚拟映射：首次访问触发缺页时按 `kind` 分配
    /// （Anonymous → 分配零页，见 [`page_fault`](Self::page_fault)）。
    ///
    /// `start` 和 `size` 必须 `PAGE_SIZE` 对齐，不得与已有映射/窗口重叠。
    /// 与 [`resolve`](Self::resolve)（查询）配对；删除随
    /// [`unmap`](Self::unmap) 原子完成（清页表 + 移除声明）。
    pub fn declare(
        &self,
        start: VirtAddr,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
    ) -> Result<(), MapError> {
        self.inner.lock().declare_durable(start, size, flags, kind)
    }

    /// 查询虚拟地址所属的映射（常数表 → 动态窗口子表）。
    ///
    /// 返回 `Option<MapInfo>`（Copy）而非引用：映射表在锁内，
    /// borrow 不能跨锁返回引用。
    pub fn resolve(&self, vaddr: VirtAddr) -> Option<MapInfo> {
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
        // `inner` 随字段自动 drop：root/tables/maps 与窗口子 Map 的帧全部归还
        // frame 池——所有权驱动，无遍历页表树、无手写 deallocate。
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

