// 地址空间 — MMU 子系统的核心抽象
//
// Space 拥有一个 Sv39 根页表与全部自有物理帧，提供虚拟→物理映射、权限管理、
// 地址翻译等高层操作。空间种类由 [`SpaceKind`] 显式区分：内核空间（ASID 0，
// 全局唯一）与用户空间（独立 ASID），构造统一走 [`SpaceBuilder`]。
//
// 簿记模型（三层，语义 = VA→PA 的显式表达）：
//   Durable — 常数侧：页表树（root: TableNode，硬件页 + 子树所有权）+ 常数映射表
//             （maps：DRAM 恒等/内核高半区、trampoline 叶 PTE、文本、trap-context 帧）
//   Dynamic  — 动态侧：堆/栈窗口，各持一个 [`BitmapAllocator`] 细分窗口内 VA，
//             每次分配的一块区间即一个子 Map（children）——heap 块 / 栈 slot
//   Map     — VA→PA 的原子单元：{ va, size, flags, kind, frames }，不变量
//             frames[i] ↔ va + i·PAGE_SIZE；帧随 Map 所有权回收（drop 归还）
//
// 用户空间不共享内核映射——trampoline 叶 PTE 只映射不拥有（帧归内核，
// Map.frames 为空）；其余帧（文本、trap-context、堆块、栈体）全归本空间所有。
// 页表树由 Durable::root（TableNode）持有：硬件页恰好 4096 B 装不下元数据，
// 树（children）放在帧外的 TableNode 上（见 table.rs）；unmap 回收变空的中间表。
//
// 锁（`Space::inner`，RelLock）：全部可变状态同锁互斥；ASID 空间为位图分配器
// 的全局实例（见 `memory::manager::asid`）。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;
use hashbrown::HashMap;

use crate::lock::OnceLock;
use crate::memory::allocator::bitmap::BitmapAllocator;
use crate::memory::allocator::frame::allocator;
use crate::{
    lock::{Level, RelLock},
    memory::PAGE_SIZE,
};

use crate::memory::manager::{
    addr::{AtomicPhysAddr, PhysAddr, VirtAddr},
    asid,
    entry::PteFlags,
    flush_asid,
    table::{Frame, FrameState, MapError, TableNode},
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
    /// 所属线程 id（线程私有映射：栈 guard/体、trap 帧）——退出时按 owner 定位回收；
    /// 共享/空间级映射（文本、DRAM、trampoline、堆块）为 `None`。
    owner: Option<usize>,
    frames: Vec<FrameState>,
}

impl Map {
    /// 构造（size 必须非零——调用方保证，见各入口的校验）。
    fn new(
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
        frames: Vec<FrameState>,
        owner: Option<usize>,
    ) -> Self {
        Self {
            va,
            size: NonZeroUsize::new(size).expect("map size must be non-zero"),
            flags,
            kind,
            owner,
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
        Ok(frame.pa())
    }

    /// 追加一帧（保持不变量；调用方保证序号连续且不越界）。
    fn push_frame(&mut self, frame: Frame) {
        debug_assert!(
            self.frames.len() < self.size.get() / PAGE_SIZE,
            "map {:#x} frame overflow",
            self.va.as_usize()
        );
        self.frames.push(FrameState::Owned(frame));
    }
}

/// 窗口种类 — 动态区域的身份，兼作 `dynamic` 表的键（`HashMap<DynamicKind, Dynamic>`）。
///
/// 数据载荷是区号（区域级窗口恒为 0）：同一种类未来可扩展多个区域（如按 NUMA
/// 节点分区的堆），键仍唯一；当前每类各一个区级窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DynamicKind {
    /// 用户堆窗口（[`USER_HEAP_BASE`] 起 [`USER_HEAP_SIZE`]）。
    Heap(usize),
    /// 任务栈窗口（[`USER_STACK_BASE`] 起 [`TASK_STACK_AREA_SIZE`]，约 5 万 slot）。
    Stack(usize),
    /// 线程 trap 帧窗口（[`FRAME_BASE`] 起 [`FRAME_REGION_SIZE`]，S-only 内核半区）。
    Frame(usize),
}

/// 动态窗口 — 固定 VA 区间 + 位图分配器 + 结构性绑定的子映射表。
///
/// `children` 记录本窗口已分配出去的每块区间（heap 块 / 栈 slot / 线程帧），
/// 与位图分配器的存活块一一对应——释放即从 children 移除（帧随 Map drop 归还），
/// 窗口可安全复用（PTE 已清、无残留映射）。身份在 `dynamic` 表的键
/// （[`DynamicKind`]）上。
#[derive(Debug)]
pub struct Dynamic {
    va: VirtAddr,
    size: NonZeroUsize,
    allocator: BitmapAllocator,
    children: Vec<Map>,
}

impl Dynamic {
    fn new(kind: DynamicKind) -> Self {
        let (base, size) = match kind {
            DynamicKind::Heap(_) => (USER_HEAP_BASE.as_usize(), USER_HEAP_SIZE),
            DynamicKind::Stack(_) => (USER_STACK_BASE.as_usize(), TASK_STACK_AREA_SIZE),
            DynamicKind::Frame(_) => (FRAME_BASE.as_usize(), FRAME_REGION_SIZE),
        };
        Self {
            va: VirtAddr::from_raw(base),
            size: NonZeroUsize::new(size).expect("window size non-zero"),
            allocator: BitmapAllocator::new(base, base + size, PAGE_SIZE),
            children: Vec::new(),
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 从位图分配一块 VA，登记一个空帧子 Map（懒分配：帧由调用方随后注入）。
    ///
    /// `owner` = 所属线程 id（私有资源如线程帧 / 栈 slot 用于退出回收定位；
    /// 共享资源如堆块传 `None`）。
    fn allocate(
        &mut self,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
        owner: Option<usize>,
    ) -> Result<MapInfo, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let (base, size) = self
            .allocator
            .allocate(size)
            .map_err(|_| MapError::OutOfMemory)?;
        let map = Map::new(
            VirtAddr::from_raw(base),
            size,
            flags,
            kind,
            Vec::new(),
            owner,
        );
        let info = map.info();
        self.children.push(map);
        Ok(info)
    }

    /// 释放一块 VA：位图精确匹配成功后移除重叠子 Map（帧随 drop 归还）。
    fn deallocate(&mut self, va: VirtAddr, size: usize) -> bool {
        if self.allocator.deallocate(va.as_usize(), size).is_err() {
            return false;
        }
        let end = va.as_usize().saturating_add(size);
        self.children.retain(|m| {
            !(va.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize())
        });
        true
    }

    /// 移除全部 `owner` 匹配的子 Map（帧随 drop 归还），返回被覆盖区间
    /// `[min_va, min_va + len)`（供调用方清 PTE 后按整区间位图归还）。
    ///
    /// 线程退出回收用：栈 slot 的守护页/栈体两子 Map 共享同一 owner，一并摘除。
    fn remove_owner(&mut self, owner: usize) -> Option<(VirtAddr, usize)> {
        let mut min = usize::MAX;
        let mut max = 0usize;
        let before = self.children.len();
        self.children.retain(|m| {
            if m.owner == Some(owner) {
                min = min.min(m.va.as_usize());
                max = max.max(m.va.as_usize().saturating_add(m.size.get()));
                false
            } else {
                true
            }
        });
        (self.children.len() != before).then(|| (VirtAddr::from_raw(min), max - min))
    }
}

/// 常数侧 — 页表树（TableNode）+ 常数映射表。
///
/// `root` 是页表树：硬件页（根/中间表，恰好一帧）+ 子树所有权（帧外 TableNode）；
/// unmap 回收变空的中间表。`maps` 覆盖空间建立期就确定、生命周期与空间一致的
/// 映射（内核 DRAM 恒等/高半区、trampoline 叶、文本、trap-context 帧）。
/// Drop 递归归还全部页表帧。
#[derive(Debug)]
pub struct Durable {
    root: TableNode,
    maps: Vec<Map>,
}

impl Durable {
    fn new() -> Result<Self, MapError> {
        Ok(Self {
            root: TableNode::root()?,
            maps: Vec::new(),
        })
    }

    /// 清 `[va, va+size)` 叶 PTE 并回收变空的中间表（unmap/dealloc/回滚共用）。
    ///
    /// 回收 = 自底向上判空（512 项全无效）即摘除子树、帧当场归还——树与 PTE
    /// 同源（`TableNode::reclaim` 先清 PTE 再摘子节点）。
    fn clear_range(&mut self, va: usize, size: usize) {
        if size == 0 {
            return;
        }
        for i in 0..size.div_ceil(PAGE_SIZE) {
            self.root.unmap(VirtAddr::from_raw(va + i * PAGE_SIZE));
        }
        // Sv39 39 位地址空间（掩去符号扩展：内核半区 VA 的 bit 39..63 高位）
        const SV39_MASK: usize = (1usize << 39) - 1;
        let end = va.saturating_add(size);
        self.root.reclaim(2, 0, va & SV39_MASK, end & SV39_MASK);
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
    fn overlaps(
        &self,
        start: VirtAddr,
        size: usize,
        windows: &HashMap<DynamicKind, Dynamic>,
    ) -> bool {
        let end = start.as_usize().saturating_add(size);
        self.maps.iter().any(|m| {
            start.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize()
        }) || windows.values().any(|w| {
            start.as_usize() < w.va.as_usize().saturating_add(w.size.get()) && end > w.va.as_usize()
        })
    }
}

/// 地址空间的可变状态 — 由 [`Space::inner`] 这把 [`RelLock`] 保护。
///
/// 只做组合：`durable`（常数侧）与 `dynamic`（堆/栈）覆盖空间全部簿记；
/// 与页表操作同锁互斥（锁约定见 [`Space`]）。
#[derive(Debug)]
struct SpaceInner {
    /// 常数侧：根页表 + 中间表帧 + 常数映射表。
    durable: Durable,
    /// 动态侧：堆 / 栈 / 帧窗口，按种类（[`DynamicKind`]）为键。
    ///
    /// no_std 的 alloc crate 无 `HashMap`，经 `hashbrown`（std 同源实现，
    /// `alloc` + `default-hasher`/foldhash 特性）提供。窗口数恒 3，查表开销无关紧要。
    dynamic: HashMap<DynamicKind, Dynamic>,
}

impl SpaceInner {
    fn new() -> Result<Self, MapError> {
        let mut dynamic = HashMap::from([
            (DynamicKind::Heap(0), Dynamic::new(DynamicKind::Heap(0))),
            (DynamicKind::Stack(0), Dynamic::new(DynamicKind::Stack(0))),
            (DynamicKind::Frame(0), Dynamic::new(DynamicKind::Frame(0))),
        ]);
        // 窗口位图构建期立即分配（不惰性）——kernel 空间的窗口簿记必须先于
        // frame 基线（record_baseline）存在：惰性推迟到首个任务分配会使 kernel
        // 空间的位图落在基线后、又随 'static 空间永不归还，check_baseline 把
        // 良性的内核窗口簿记误报为任务帧泄漏（1 GiB 栈窗口 → 32 KiB 位图 →
        // frame 分配器 1 块）。
        for w in dynamic.values_mut() {
            w.allocator.eager().map_err(|_| MapError::OutOfMemory)?;
        }
        Ok(Self {
            durable: Durable::new()?,
            dynamic,
        })
    }

    /// 查询 `vaddr` 所属的映射（常数表 → 动态窗口子表），返回 Copy 摘要。
    fn resolve(&self, vaddr: VirtAddr) -> Option<MapInfo> {
        if let Some(m) = self.durable.resolve(vaddr) {
            return Some(m.info());
        }
        for w in self.dynamic.values() {
            if w.contains(vaddr)
                && let Some(m) = w.children.iter().rev().find(|m| m.contains(vaddr))
            {
                return Some(m.info());
            }
        }
        None
    }

    /// 查询 `vaddr` 所属映射的可变引用（缺页注入帧用）。
    fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        if let Some(m) = self.durable.resolve_mut(vaddr) {
            return Some(m);
        }
        for w in self.dynamic.values_mut() {
            if w.contains(vaddr)
                && let Some(m) = w.children.iter_mut().rev().find(|m| m.contains(vaddr))
            {
                return Some(m);
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
            .push(Map::new(start, size, flags, kind, Vec::new(), None));
        Ok(())
    }

    /// 页表读翻译（内部版，调用者须持锁，与 map/unmap 写互斥）。
    fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.durable
            .root
            .walk_ref(vaddr)
            .map(|x| (x.0 + vaddr.offset(), x.1))
            .ok()
    }
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
/// 由 [`Dynamic`] 的位图分配器实例消费。
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

/// trampoline 页的物理地址（纯链接符号：内核镜像恒等加载，链接地址即物理地址）。
///
/// 属内核空间布局（TRAMPOLINE VA 所映射的物理帧），故与 `TRAMPOLINE` 同驻本模块；
/// `unit::init` 映射该页时经此取 PA。`runtime::trampoline` 的
/// `__alltraps/__restore` 仍在运行时层按自身 VA 偏移计算。
pub(crate) fn trampoline_pa() -> PhysAddr {
    unsafe extern "C" {
        static __trampoline_start: u8;
    }
    PhysAddr::from_raw(core::ptr::addr_of!(__trampoline_start) as usize)
}

/// per-hart 内核帧槽数（= 内核帧区 VA 窗口宽度 = MAX_HART_SLOTS，编译期预留
/// 4096 页 = 16 MiB 高位虚拟地址）。帧区与帧窗口布局耦合，见下方布局断言——
/// 实际按 hart_count() 映射/填充，槽位预留使 __strap 的 TP 索引不依赖 DTB
/// 核数，仅多占高位虚拟地址空间（虚拟地址免费，窗口宽度与核数解耦）。
pub(crate) const KERNEL_FRAME_SLOTS: usize = crate::machine::MAX_HART_SLOTS;

/// per-hart 内核帧区基址（紧贴 TRAMPOLINE 之下 SLOTS 页；hart h 帧 VA =
/// KERNEL_FRAME_BASE + h·PAGE_SIZE）。帧由 unit::init 逐页映射（PA 存
/// KERNEL_FRAMES[h]），__strap 经 KERNEL_FRAMES_LUI + tp 索引。
pub(crate) const KERNEL_FRAME_BASE: VirtAddr =
    VirtAddr::from_raw(TRAMPOLINE.as_usize() - KERNEL_FRAME_SLOTS * PAGE_SIZE);

/// 线程 trap 帧窗口大小（64 MiB − 内核帧区；帧窗口止于 KERNEL_FRAME_BASE）。
/// 4096 槽帧区使窗口缩到 ≈48 MiB（0x300_1000 ≈ 12289 个 4 KiB 帧）。
pub(crate) const FRAME_REGION_SIZE: usize = 0x400_0000 - (KERNEL_FRAME_SLOTS - 1) * PAGE_SIZE;
/// 线程 trap 帧窗口基址（`KERNEL_FRAME_BASE` 之下，内核半区，S-only——用户不可触碰）。
pub(crate) const FRAME_BASE: VirtAddr =
    VirtAddr::from_raw(KERNEL_FRAME_BASE.as_usize() - FRAME_REGION_SIZE);

// ── 地址空间总览（Sv39）───────────────────────────────────────────
//
// 用户半区（bit 38 = 0，VPN[2] = 0..255）：
//
//   0x0000_0000
//     ┌─────────────────────────────────┐
//     │             保留                │
//     ├─────────────────────────────────┤ 0x2000_0000  USER_HEAP_BASE
//     │  用户堆窗口 64 MiB              │ USER_HEAP_BASE + USER_HEAP_SIZE
//     │ （BitmapAllocator 实例管理）    │
//     ├─────────────────────────────────┤ 0x2040_0000
//     │             保留                │
//     ├─────────────────────────────────┤ 0xC000_0000  USER_STACK_BASE
//     │  任务栈窗口 1 GiB               │ USER_STACK_BASE + TASK_STACK_AREA_SIZE
//     │ （16 KiB 栈 + 守护页，约 5 万） │
//     ├─────────────────────────────────┤ 0x1_0000_0000（低 4 GiB，设计上限）
//     │             保留（至 2^38）     │
//     └─────────────────────────────────┘
//
// 内核半区（bit 38 = 1，VPN[2] = 256..511，起点 KERNEL_BASE）：
//
//   0xFFFF_FFFF_FFFF_F000 — TRAMPOLINE（页对齐；__strap 经 KERNEL_FRAMES_LUI 注入）
//   0xFFFF_FFFF_FFEF_F000 ┬ per-hart 内核帧区：KERNEL_FRAME_BASE 起 4096 页（16 MiB VA 窗口）
//   0xFFFF_FFFF_FFFF_F000 ┘ （hart h 帧 = BASE + h·PAGE；__strap 按 TP 索引）
//   0xFFFF_FFFF_FCEF_E000 — FRAME_BASE（帧窗口下界）
//     ┌─────────────────────────────────┐
//     │  线程 trap 帧窗口（≈48 MiB）    │ FRAME_BASE + FRAME_REGION_SIZE = KERNEL_FRAME_BASE
//     │（≈12K 并发帧，S-only，位图管理）│
//     └─────────────────────────────────┘ KERNEL_FRAME_BASE
//
// 布局即不变量：以下断言把「对齐 / 相邻 / 不重叠」编译期锁死——
// 改布局必须先改这里（编译器兜底），并同步 link.ld / trampoline 汇编。
const _: () = {
    // 注意：VirtAddr 的 Add/Sub/PartialEq 非 const fn，此处一律用 as_usize() 裸算术。
    assert!(TRAMPOLINE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(KERNEL_FRAME_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    // 内核帧区：KERNEL_FRAME_BASE 起 SLOTS 页，恰止于 TRAMPOLINE（相邻、不重叠）
    assert!(KERNEL_FRAME_BASE.as_usize() + KERNEL_FRAME_SLOTS * PAGE_SIZE == TRAMPOLINE.as_usize());
    // 帧窗口：页对齐、恰止于 KERNEL_FRAME_BASE（内核帧区在其上方，互不重叠）
    assert!(FRAME_REGION_SIZE.is_multiple_of(PAGE_SIZE));
    assert!(FRAME_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(FRAME_BASE.as_usize() + FRAME_REGION_SIZE == KERNEL_FRAME_BASE.as_usize());
    assert!(KERNEL_BASE.as_usize() == 0xFFFF_FFC0_0000_0000); // Sv39 内核半区起点（VPN[2] = 256）
    assert!(USER_HEAP_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(USER_HEAP_SIZE.is_multiple_of(PAGE_SIZE));
    assert!(USER_HEAP_BASE.as_usize() + USER_HEAP_SIZE <= USER_STACK_BASE.as_usize()); // 堆窗口不越过栈窗口
    assert!(USER_STACK_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(TASK_STACK_AREA_SIZE.is_multiple_of(PAGE_SIZE));
    assert!(TASK_STACK_SIZE.is_multiple_of(PAGE_SIZE));
    assert!(USER_STACK_BASE.as_usize() + TASK_STACK_AREA_SIZE <= 0x1_0000_0000); // 栈窗口不越出低 4 GiB
};
/// Sv39 虚拟地址空间。
///
/// 拥有根页表与**全部自有物理帧**。用户空间只映射 trampoline 叶 PTE（帧归内核、
/// 不拥有）与自有 trap-context 帧，不复制/共享内核映射。全部可变状态
/// （`SpaceInner`：durable / dynamic）收进一把 [`RelLock`]；根页表随
/// `durable.root` 持有（Box 自动 drop）。
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
/// 需要同时借 `durable` 的不同字段（如 `root` 与 `maps`）时，先绑定局部
/// 变量（字段级拆借），再调用方法。
///
/// # Drop
///
/// `durable.root`（页表树，递归归还全部表帧）、`durable.maps` 与 `dynamic` 窗口的
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
                asid: asid::allocate(),
            },
        }
    }

    /// 完成构建：分配根页表帧；用户空间额外从内核空间种入 trampoline 叶
    /// PTE（见 [`SpaceBuilder::seed_user`]）。线程 trap 帧由后续
    /// [`Space::frame_alloc`] 按线程分配。
    ///
    /// # Errors
    ///
    /// 物理帧耗尽时返回 [`MapError::OutOfMemory`]。
    pub fn build(self) -> Result<Space, MapError> {
        let mut space = Space {
            kind: self.kind,
            inner: RelLock::new_level(Level::Space, SpaceInner::new()?),
        };
        if matches!(space.kind, SpaceKind::User { .. }) {
            self.seed_user(&mut space)?;
        }
        Ok(space)
    }

    /// 从内核地址空间出用户空间（`build()` 内部调用）。
    ///
    /// 不复制内核半区映射——用户页表只含用户映射 + trampoline 叶 PTE 复制
    /// （[`Space::TRAMPOLINE`] VA → 内核 trampoline 物理页，**不拥有**，以 Reserved
    /// 常数映射登记：借用、无帧）。
    ///
    /// 线程 trap 帧在此不创建：每线程帧由 [`Space::frame_alloc`] 在 Frame
    /// 窗口分配（VA 任意、S-only），内核切换元数据在 spawn 时从内核帧拷贝。
    ///
    /// # Errors
    ///
    /// 页表帧耗尽时返回 [`MapError::OutOfMemory`]。
    fn seed_user(&self, space: &mut Space) -> Result<(), MapError> {
        // 读内核空间的 trampoline 叶 PTE（内核空间唯一归属 KERNEL_TEAM，只读）
        let (tramp_pa, tramp_flags) = {
            let ks_inner = crate::work::unit::team::kernel().space.inner.lock();
            ks_inner.durable.root.walk_ref(TRAMPOLINE)?
        };

        // trampoline 叶（帧归内核，借用：Reserved 常数映射、无帧）
        space.map(
            TRAMPOLINE,
            tramp_pa,
            PAGE_SIZE,
            tramp_flags,
            MapKind::Reserved,
            Vec::new(),
        )?;
        Ok(())
    }
}

/// 逐帧装叶子 PTE：VA 连续推进、PA 取每帧自身（物理可断，不假设连续）。
///
/// [`attach_durable`](Space::attach_durable) 与
/// [`attach_dynamic`](Space::attach_dynamic) 共享的同一段循环——把一帧 Physical
/// 页装配到对应 VA 的叶子 PTE（新中间表由页表树 [`TableNode`] 内部持有）。
/// 簿记（登记 / 压入 Map）由调用方各自完成，故本函数**仅装 PTE、不动簿记**。
fn install_frame_ptes(
    root: &mut TableNode,
    va: VirtAddr,
    frames: &[Frame],
    flags: PteFlags,
) -> Result<(), MapError> {
    for (i, frame) in frames.iter().enumerate() {
        let pa = PhysAddr::from_raw(frame.as_ptr() as usize);
        let addr = va + i * PAGE_SIZE;
        root.map(addr, pa, PAGE_SIZE, flags)?;
    }
    Ok(())
}

impl Space {
    // ── 映射操作 ──────────────────────────────────────────────

    /// 分配一个任务栈窗口 slot，返回栈体 VA（16 KiB，向下增长，底部守护页）。
    ///
    /// slot = 守护页 + 栈体，一次从 Stack 窗口位图领取；守护页登记为 Reserved
    /// 子 Map——栈溢出触碰守护页时 fault 处理器识别为「预留映射访问」而非笼统的
    /// 「无 Map」。栈体登记为 Anonymous 子 Map（帧随后经
    /// [`attach_dynamic`](Self::attach_dynamic) 注入）。两子 Map 均标 `owner`，退出时
    /// 按 owner 一并回收（[`task_reclaim`](Self::task_reclaim)）。
    pub(crate) fn stack_allocate(&self, owner: usize) -> Result<VirtAddr, MapError> {
        let mut inner = self.inner.lock();
        let slot_size = TASK_STACK_SIZE + STACK_GUARD_SIZE;
        let stack = {
            let windows: &mut HashMap<DynamicKind, Dynamic> = &mut inner.dynamic;
            let kind = DynamicKind::Stack(0);
            windows.get_mut(&kind).expect("window exists")
        };
        let (slot_va, _) = stack
            .allocator
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
            Some(owner),
        ));
        // 栈体 [slot_va + GUARD, +TASK_STACK_SIZE)：Anonymous，帧待 attach。
        // U 位按空间种类：用户空间栈需 U（用户 push）；内核空间栈（kernel 团队/kthread）
        // 不得带 U——S 态 SUM=0 下访问 U 页会页故障，而内核任务跑 S 态。
        let body_flags = if matches!(self.kind, SpaceKind::Kernel) {
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D
        } else {
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D
        };
        let body_va = slot_va + STACK_GUARD_SIZE;
        stack.children.push(Map::new(
            body_va,
            TASK_STACK_SIZE,
            body_flags,
            MapKind::Anonymous,
            Vec::new(),
            Some(owner),
        ));
        Ok(body_va)
    }

    /// 把帧注入映射到一段**已登记的动态窗口子 Map**（spawn 用：先经窗口位图
    /// `allocate` 预留 VA 块获得子 Map，再分配帧 attach）。
    ///
    /// [`attach_durable`](Self::attach_durable) 的 dynamic 簿记变体：逐帧装 PTE
    /// （[`install_frame_ptes`]，中间表由页表树 TableNode 持有）+ `push_frame`
    /// （入窗口子 Map，保持「帧 i ↔ va + i·PAGE_SIZE」不变量）。中途失败返回
    /// 错误，调用方 drop Space 统一回收（已装 PTE 与已入帧随空间归还）。
    pub(crate) fn attach_dynamic(&self, va: VirtAddr, frames: Vec<Frame>) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        // 各窗口 VA 区间互不重叠（布局断言保证），故按 VA 定位子 Map 唯一。
        let child = dynamic
            .values_mut()
            .flat_map(|w| w.children.iter_mut())
            .find(|m| m.va == va)
            .ok_or(MapError::NoRegion)?;
        let child_flags = child.flags;
        install_frame_ptes(&mut durable.root, va, &frames, child_flags)?;
        for frame in frames {
            child.push_frame(frame);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 分配一个线程 trap 帧：Frame 窗口位图取一页 VA → 分配物理帧 → 装 PTE
    /// （S-only，内核半区——用户不可触碰）→ 登记帧子 Map（`owner` = 线程 id，
    /// 退出时按 owner/VA 回收）。返回 `(帧 VA, 帧 PA)`。
    ///
    /// 帧元数据（内核切换信息 + 用户上下文）由调用方（spawn_thread）随后经 PA
    /// 填充。帧 VA 无固定地址——`__alltraps`/`__restore` 经帧内 `self_va` 定位，
    /// 是每线程帧可任意放置的机制前提。
    ///
    /// # Errors
    ///
    /// 窗口耗尽或物理帧耗尽 → [`MapError::OutOfMemory`]（后者回滚已装状态）。
    pub(crate) fn frame_allocate(&self, owner: usize) -> Result<(VirtAddr, PhysAddr), MapError> {
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D; // S-only
        let frame_win = {
            let kind = DynamicKind::Frame(0);
            dynamic.get_mut(&kind).expect("window exists")
        };
        let info = frame_win.allocate(PAGE_SIZE, flags, MapKind::Anonymous, Some(owner))?;
        let page =
            Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
        let pa = PhysAddr::from_raw(page.as_ptr() as usize);
        if durable.root.map(info.va, pa, PAGE_SIZE, flags).is_err() {
            // 回滚：清可能残留的中间表/PTE + VA 退回位图（空子 Map 一并移除）
            durable.clear_range(info.va.as_usize(), PAGE_SIZE);
            frame_win.deallocate(info.va, PAGE_SIZE);
            drop(inner);
            // SAFETY: S-mode 下 sfence.vma 恒合法。
            unsafe {
                flush_asid(self.kind.asid());
            }
            return Err(MapError::OutOfMemory);
        }
        let child = frame_win
            .children
            .iter_mut()
            .find(|m| m.va == info.va)
            .expect("frame child exists");
        child.push_frame(page);
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok((info.va, pa))
    }

    /// 回收一个线程的全部私有资源（栈 slot + trap 帧），`owner` = 线程 id。
    ///
    /// 线程退出调用（SCHEDULER 锁内）：
    /// 1. 栈窗口按 owner 摘除守护页/栈体子 Map（栈帧随 drop 归还 frame 池）；
    /// 2. 清整 slot 叶 PTE + 回收变空的中间表；
    /// 3. 位图归还 slot VA（[`BitmapAllocator::deallocate`] 精确匹配）；
    /// 4. 帧窗口按帧 VA 摘除帧子 Map（帧页归还）→ 清 PTE → 位图归还帧 VA。
    ///
    /// **VA 复用安全的关键**：PTE 清理 + [`flush_asid`] 先于归还，杜绝复用者
    /// 摸到旧线程残留映射。
    pub(crate) fn task_reclaim(&self, owner: usize, frame_va: VirtAddr) {
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        // 1+3. 栈窗口：摘 owner 子 Map → 清 PTE → 位图归还 slot
        let stack = {
            let kind = DynamicKind::Stack(0);
            dynamic.get_mut(&kind).expect("window exists")
        };
        if let Some((slot_va, slot_size)) = stack.remove_owner(owner) {
            durable.clear_range(slot_va.as_usize(), slot_size);
            // 位图归还与子 Map 结构性绑定：remove_owner 命中则 dealloc 必成功
            let _ = stack.allocator.deallocate(slot_va.as_usize(), slot_size);
        }
        // 2+4. 帧窗口：摘帧子 Map → 清 PTE → 位图归还帧 VA
        let frames = {
            let kind = DynamicKind::Frame(0);
            dynamic.get_mut(&kind).expect("window exists")
        };
        if frames.deallocate(frame_va, PAGE_SIZE) {
            durable.clear_range(frame_va.as_usize(), PAGE_SIZE);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
    }

    /// 用户堆分配：经 Heap 窗口位图保留页对齐 VA 块，登记空子 Map（Anonymous），
    /// 逐页从 frame 分配器取物理页映射到用户区（U|R|W）并注入子 Map。返回分配 VA。
    ///
    /// 堆窗口固定 [`USER_HEAP_BASE`] 起 [`USER_HEAP_SIZE`]；窗口耗尽 →
    /// [`MapError::OutOfMemory`]。立即分配（非懒分配）：教学简化，页表与物理页
    /// 当场就位，用户访问不再缺页。中途帧耗尽时回滚：清已映射页叶子 + 移除子
    /// Map（帧随 drop 归还）+ VA 块退回复用（[`BitmapAllocator::deallocate`]；
    /// 中间表帧已由 clear_range 回收）。
    pub(crate) fn heap_allocate(&self, size: usize) -> Result<VirtAddr, MapError> {
        let mut inner = self.inner.lock();
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        let SpaceInner { durable, dynamic } = &mut *inner;
        // 1. 窗口位图保留块 + 登记空子 Map（Anonymous）
        let heap = {
            let kind = DynamicKind::Heap(0);
            dynamic.get_mut(&kind).expect("window exists")
        };
        let info = heap.allocate(size, flags, MapKind::Anonymous, None)?;
        // 2. 逐页分配物理帧并映射（立即分配）+ 注入子 Map
        let pages = info.size.get() / PAGE_SIZE;
        let mut mapped = 0usize;
        while mapped < pages {
            let page = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            let va = info.va + mapped * PAGE_SIZE;
            if durable.root.map(va, pa, PAGE_SIZE, flags).is_err() {
                // 回滚：清已映射页叶子并回收中间表 + 移除子 Map（帧随 drop 归还）+ VA 退回位图
                durable.clear_range(info.va.as_usize(), mapped * PAGE_SIZE);
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
        // audit: 用户堆活块入账（alloc-site；用户侧清零语义，不 poison/canary）。
        // 键 = asid << 32 | VA：多个空间共享同一堆窗口 VA，须并入空间身份防碰撞。
        #[cfg(all(debug_assertions, feature = "audit"))]
        crate::memory::integrity::LEDGER.mark(
            (self.kind.asid() << 32) | info.va.as_usize(),
            info.size.get(),
            crate::lock::ra(),
            crate::memory::integrity::OwnerKind::UserHeap,
        );
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(info.va)
    }

    /// 用户堆释放：位图精确匹配 `(addr, size)` 后清叶子 PTE（含回收中间表）并移除子 Map（帧随
    /// drop 归还 frame 池）。返回是否找到并释放（未分配/部分已释放的区间返回
    /// false，同旧块表精确匹配语义）。
    pub(crate) fn heap_deallocate(&self, addr: VirtAddr, size: usize) -> bool {
        let mut inner = self.inner.lock();
        let SpaceInner { durable, dynamic } = &mut *inner;
        let heap = {
            let kind = DynamicKind::Heap(0);
            dynamic.get_mut(&kind).expect("window exists")
        };
        // 1. 位图精确匹配释放 + 移除子 Map（未分配/部分已释放 → 返回 false）
        if !heap.deallocate(addr, size) {
            return false;
        }
        // 2. 清叶子 PTE + 回收变空的中间表（帧已随子 Map 移除 drop 归还）
        durable.clear_range(addr.as_usize(), size);
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        // audit: 注销用户堆账目（无账 = 悬垂/双释放现行；键与 heap_allocate 相同）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        crate::memory::integrity::LEDGER.unmark((self.kind.asid() << 32) | addr.as_usize(), size);
        true
    }

    /// 映射 `size` 字节虚拟地址到物理地址（常数映射唯一公共入口）。
    ///
    /// PTE 安装 + 登记常数 [`Map`]（VA→PA 簿记）一次完成：`kind` 决定缺页响应，
    /// `frames` 为本映射**拥有**的帧（借用映射——DRAM 恒等、trampoline 叶——传空）。
    /// 按需分配中间页表——由页表树（[`Durable::root`]）持有。
    ///
    /// **vaddr、paddr、size 必须全部按 [`PAGE_SIZE`] 对齐**，且不得与已有映射/
    /// 窗口区间重叠。非对齐大小的调用方（如 MMIO 设备映射）须自行向上取整。
    ///
    /// 本空间所有页表帧私有，`map` 直接写叶子，无需 COW。
    ///
    /// # Errors
    ///
    /// 参见 [`TableNode::map`] 与 [`MapError::AlreadyMapped`]。
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
        if size == 0 || vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        // 1. 重叠检查（常数表 + 窗口区间）
        if inner.durable.overlaps(vaddr, size, &inner.dynamic) {
            return Err(MapError::AlreadyMapped);
        }
        // 2. PTE 安装 + 3. 登记常数映射（一次解构拿 durable 各字段，字段间可拆借）
        let SpaceInner { durable, .. } = &mut *inner;
        durable.root.map(vaddr, paddr, size, flags)?;
        durable.maps.push(Map::new(
            vaddr,
            size,
            flags,
            kind,
            frames.into_iter().map(FrameState::Owned).collect(),
            None,
        ));
        drop(inner);
        // 按本空间 ASID 局部刷：只失效本地址空间的旧条目，其它任务 TLB 保留。
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 把调用方配好的物理帧映射到一段连续虚拟地址（**常数侧**公共入口之一）。
    ///
    /// 与 [`map`](Self::map) 的差别只在物理连续性：`map` 假定 `paddr` 起 `size`
    /// **物理连续**（一次装整段，见 [`TableNode::map`] 的 `pa = paddr + i·PAGE`），
    /// 而本方法逐帧装 PTE（[`install_frame_ptes`]）——每帧用自身 PA，物理可断，
    /// 与 [`attach_dynamic`](Self::attach_dynamic) 同范本。
    ///
    /// 场景：程序段装载（[`crate::work::unit::loader`]）、用户堆——帧是独立
    /// 堆分配的 `Box`，物理**不连续**。若只用 `map`，第 i 页 PTE 会被算成
    /// `pa0 + i·PAGE_SIZE`，指向错误的物理页（`TableNode::map` 的物理连续假设
    /// 在这里失效）。簿记仍登记**一张**多页 [`Map`]：其不变量「帧 i ↔
    /// va + i·PAGE」只约束 VA 索引、不要求 PA 连续，帧随 Map 所有权回收。
    ///
    /// `vaddr` 必须按页对齐；`frames` 非空，长度即页数（`size = len·PAGE_SIZE`）。
    ///
    /// # Errors
    ///
    /// 参见 [`TableNode::map`] 与 [`MapError::AlreadyMapped`]（重叠/帧耗尽，
    /// 均不改动空间；中途失败时调用方按既有契约 drop 整个 Space 统一回收）。
    pub(crate) fn attach_durable(
        &self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
        kind: MapKind,
    ) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let pages = frames.len();
        // 0. 参数校验：非空帧 + VA 页对齐（先于任何页表修改）
        if pages == 0 || vaddr.offset() != 0 {
            return Err(MapError::NotAligned);
        }
        let size = pages * PAGE_SIZE;
        // 1. 重叠检查（常数表 + 窗口区间）
        if inner.durable.overlaps(vaddr, size, &inner.dynamic) {
            return Err(MapError::AlreadyMapped);
        }
        // 2. 逐帧装 PTE（每帧自身 PA，物理可断）+ 3. 登记一张多页常数映射
        let SpaceInner { durable, .. } = &mut *inner;
        install_frame_ptes(&mut durable.root, vaddr, &frames, flags)?;
        durable.maps.push(Map::new(
            vaddr,
            size,
            flags,
            kind,
            frames.into_iter().map(FrameState::Owned).collect(),
            None,
        ));
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
    /// 页表侧清叶子 PTE 并**回收变空的中间表**（表帧当场归还 frame 池）；簿记侧移除被 `[start,
    /// start+size)` **完全覆盖**的映射（帧随 drop 归还——unmap 即释放）。部分
    /// 重叠的映射保留记录、仅清 PTE：munmap 边界拆分留待后端接入时实现。
    /// `vaddr`/`size` 不要求页对齐（向上取整语义与 POSIX munmap 一致）。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        let end = vaddr.as_usize().saturating_add(size);

        let mut inner = self.inner.lock();
        // 页表侧：清叶子 PTE + 回收变空的中间表
        inner.durable.clear_range(vaddr.as_usize(), size);
        // 簿记侧：移除被完全覆盖的常数映射与窗口子 Map
        let covered = |m: &Map| {
            vaddr.as_usize() <= m.va.as_usize()
                && end >= m.va.as_usize().saturating_add(m.size.get())
        };
        inner.durable.maps.retain(|m| !covered(m));
        for w in inner.dynamic.values_mut() {
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
            let leaf = root.walk_mut(va, false)?;
            leaf.set_flags(flags | PteFlags::V);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    // ── COW（copy-on-write 共享帧）────────────────────────────
    //
    // borrow ↔ unborrow 空间级成对（共享/离共享）；to_mut / into_owned
    // 写触发/主动的转私有。仅作用 durable 常数侧（程序文本/数据/BSS）；
    // 栈/帧/堆窗口为线程私有、永不 COW 共享。跨空间分发 Arc 属 fork，不在此。

    /// 把 `[start, start+size)` 内可写页提升为共享只读：Owned → Borrowed(Arc)，
    /// PTE 重指到 Arc 帧、置 A/D、清 W（保留 R，只拦写）。
    ///
    /// # Errors
    ///
    /// 区间任一页未映射或非可写 → [`MapError::NotMapped`]。
    #[allow(dead_code)] // fork 后端预留
    pub fn borrow(&self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        for i in 0..size.div_ceil(PAGE_SIZE) {
            let va = start + i * PAGE_SIZE;
            let (bytes_src, flags) = {
                let SpaceInner { durable, .. } = &*inner;
                let map = durable
                    .maps
                    .iter()
                    .find(|m| m.contains(va))
                    .ok_or(MapError::NotMapped)?;
                let idx = (va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                match &map.frames[idx] {
                    FrameState::Borrowed(_) => continue,
                    FrameState::Owned(b) => {
                        let bytes: &[u8] = &**b;
                        (bytes, map.flags)
                    }
                }
            };
            // Arc 从 frame 分配器新建（带分配器参数）+ 拷字节
            let mut arc: Arc<[u8; PAGE_SIZE], &'static dyn alloc::alloc::Allocator> =
                Arc::new_in([0u8; PAGE_SIZE], allocator());
            Arc::get_mut(&mut arc)
                .expect("fresh arc")
                .copy_from_slice(bytes_src);
            let arc_pa = PhysAddr::from_raw(Arc::as_ptr(&arc) as usize);
            // 换帧：旧 Owned 归还，PTE 重指到 Arc 帧、清 W
            {
                let SpaceInner { durable, .. } = &mut *inner;
                let map = durable
                    .maps
                    .iter_mut()
                    .find(|m| m.contains(va))
                    .ok_or(MapError::NotMapped)?;
                let idx = (va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                let old = core::mem::replace(&mut map.frames[idx], FrameState::Borrowed(arc));
                drop(old); // 旧 Owned 帧归还 frame 池
                let leaf = durable.root.walk_mut(va, false)?;
                let ppn = (arc_pa.as_usize() >> 12) as u64;
                leaf.set(
                    ppn,
                    (flags & !PteFlags::W) | PteFlags::A | PteFlags::D | PteFlags::V,
                );
            }
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 写缺页分裂：保证该页私有可写。Borrowed → 分新 Owned 拷字节、装带 W PTE、
    /// drop 本空间 Arc；Owned → 仅确保 W 置位（no-op 语义）。
    ///
    /// # Errors
    ///
    /// 页未映射 → [`MapError::NotMapped`]；帧耗尽 → [`MapError::OutOfMemory`]。
    pub fn to_mut(&self, va: VirtAddr) -> Result<(), MapError> {
        let page = va.page_align();
        let mut inner = self.inner.lock();
        {
            let SpaceInner { durable, .. } = &mut *inner;
            let map = durable
                .maps
                .iter_mut()
                .find(|m| m.contains(page))
                .ok_or(MapError::NotMapped)?;
            let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
            let flags = map.flags;
            if let FrameState::Owned(_) = &map.frames[idx] {
                let leaf = durable.root.walk_mut(page, false)?;
                leaf.set_flags(leaf.flags() | PteFlags::W | PteFlags::V);
                return Ok(());
            }
            let new_box: Frame = {
                let FrameState::Borrowed(arc) = &map.frames[idx] else {
                    unreachable!("checked above")
                };
                let mut nb = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                    .map_err(|_| MapError::OutOfMemory)?;
                nb.copy_from_slice(&arc[..]);
                nb
            };
            let pa = PhysAddr::from_raw(new_box.as_ptr() as usize);
            let old = core::mem::replace(&mut map.frames[idx], FrameState::Owned(new_box));
            drop(old); // 放下共享 Arc（计数 −1）
            let leaf = durable.root.walk_mut(page, false)?;
            let ppn = (pa.as_usize() >> 12) as u64;
            leaf.set(ppn, flags | PteFlags::W | PteFlags::V);
        }
        drop(inner);
        // SAFETY: S-mode 下 sfence.vma 恒合法。
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 无条件私有化：立即分裂成私有 Owned（fork 后父方脱离共享用）。
    /// 语义 = to_mut（保证该页从此私有可写）。
    #[allow(dead_code)] // fork 后端预留
    pub fn into_owned(&self, va: VirtAddr) -> Result<(), MapError> {
        self.to_mut(va)
    }

    /// 判别 va 所在页是否 Borrowed 共享态（未映射/未持有帧 → false）。
    pub fn is_borrowed(&self, va: VirtAddr) -> bool {
        let page = va.page_align();
        let inner = self.inner.lock();
        let SpaceInner { durable, .. } = &*inner;
        let Some(map) = durable.maps.iter().find(|m| m.contains(page)) else {
            return false;
        };
        let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
        matches!(map.frames.get(idx), Some(FrameState::Borrowed(_)))
    }

    /// 放下 `[start, start+size)` 内共享引用（Borrowed 页 Arc 计数 −1、归零归还）。
    ///
    /// 实现注记：本空间对该帧的 Arc 引用由 `Map.frames` 持有——卸映射
    /// （unmap / task_reclaim / Space::drop）drop 整个 Map 时随帧自动释放，
    /// 无需额外操作。本方法保留为 borrow 的反向语义锚点，本身不改变状态；
    /// 必须与 PTE 拆除路径（unmap）配对，否则共享页会悬空。
    #[allow(dead_code)] // 语义由 Map teardown 承担；borrow 成对锚点
    pub fn unborrow(&self, start: VirtAddr, size: usize) {
        let _ = (&self, start, size);
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
                durable.root.map(va, pa, PAGE_SIZE, flags)?;
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

    /// 簿记↔页表一致性审计（boot / 压力测试后调用；不一致即 panic）。
    /// 每个自持 Map 的每帧：translate(va + i·PAGE) 必须在页表中且 PA 与帧相等。
    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        let inner = self.inner.lock();
        let check = |m: &Map| {
            for (i, f) in m.frames.iter().enumerate() {
                let va = m.va + i * PAGE_SIZE;
                let expect = f.pa();
                match inner.translate(va) {
                    Some((pa, _)) if pa == expect => {}
                    other => panic!(
                        "space audit @{:#x}: pte {other:?} != frame {expect:#x} (map {:#x}+{})",
                        va.as_usize(),
                        m.va.as_usize(),
                        m.size.get()
                    ),
                }
            }
        };
        inner.durable.maps.iter().for_each(check);
        for w in inner.dynamic.values() {
            w.children.iter().for_each(check);
        }
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
        inner.durable.root.ppn()
    }

    /// 返回本空间的 ASID（写入 `satp.ASID` 用；0 = 内核空间）。
    pub fn asid(&self) -> usize {
        self.kind.asid()
    }

    /// 页表树节点总数（根 + 全部子孙；debug 统计/PT 回收自测用）。
    #[cfg(debug_assertions)]
    pub fn table_count(&self) -> usize {
        self.inner.lock().durable.root.count()
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
            asid::deallocate(asid);
        }
        // `inner` 随字段自动 drop：root（页表树，递归归还全部表帧）/maps 与窗口子 Map 的帧全部归还
        // frame 池——所有权驱动，无遍历页表树、无手写 deallocate。
    }
}

// ── 内核地址空间 ─────────────────────────────────────────────

/// per-hart 内核帧物理地址表（`unit::init` 按实际核数**动态分配**并发布；
/// `__strap` 按 TP 索引的是**帧区 VA**（KERNEL_FRAME_BASE，LUI 常量注入），
/// 不读此表；本表仅供 Rust 侧经 [`kernel_frame_pa`] 读 PA——trap::init 写帧
/// 元数据、init_hart 取 hart 0 框架。仿 SCHEDULERS/TRAP_STACKS：
/// Box::leak 进 OnceLock，先于任何帧映射分配）。
static KERNEL_FRAMES: OnceLock<&'static [AtomicPhysAddr]> = OnceLock::new();

/// 按实际核数分配帧 PA 表（`unit::init` 调用，恰好一次，先于任何帧映射）。
pub(crate) fn init_kernel_frames(n: usize) {
    let table: Box<[AtomicPhysAddr]> = (0..n)
        .map(|_| AtomicPhysAddr::new(PhysAddr::from_raw(0)))
        .collect();
    assert!(
        KERNEL_FRAMES.set(Box::leak(table)).is_ok(),
        "kernel frames double init"
    );
}

/// 帧 PA 表只读视图（`unit::init` 映射 per-hart 帧时迭代用）。
pub(crate) fn kernel_frames() -> &'static [AtomicPhysAddr] {
    KERNEL_FRAMES.get().expect("kernel frames not initialized")
}

/// hart h 的内核帧物理地址（__strap 按 TP 索引的帧页 VA；trap::init 写元数据）。
///
/// hart 0 的帧供用户帧构建从它拷贝内核切换信息，统一经
/// `kernel_frame_pa(hart)` 读取。
pub fn kernel_frame_pa(hart: usize) -> PhysAddr {
    kernel_frames()[hart].load(Ordering::Relaxed)
}
