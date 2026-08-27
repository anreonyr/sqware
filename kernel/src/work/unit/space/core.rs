// core — 主类型 [`Space`] / [`SpaceBuilder`] / [`Segments`] + 业务流程
// （含 [`SpaceInner`] 组合层 + 锁约定）。
//
// # SpaceInner 锁约定（`Space::inner`，RelLock）
//
// 全部可变状态由 `RelLock` 互斥（跨 hart 真自旋，同 hart 可重入——多核下两个 hart
// 共享同一空间做 map/unmap/缺页时互斥；同 hart 持锁期间同步缺页（异步中断不受
// SIE 屏蔽）可重入）。`kind` 分配后不可变，故 [`Space`] 可放心跨线程传递
//（紧跟本节的 unsafe impl Send/Sync）。
//
// 锁约定：窗口事务统一经 [`Space::with`] / [`Space::with_flush`]（锁恰好一次，
// 闭包内直接操作 `inner`）；其余公开方法各自锁恰好一次、不重入——重入时若两个
// guard 同时 DerefMut 会构成 `&mut` 别名（UB）。`with` 闭包内**不得再调用**
// `Space` 的任何方法（同样构成重入双 guard）。
//
// 页表树读写与 `SpaceInner` 数据共享同一把锁：`translate` 读页表、`map`/`unmap`
// 写页表，都要持锁互斥（页表修改跨核可见性由锁的 Release/Acquire 保证）。
//
// 借用约定：guard 是 `Deref`，方法调用的自动引用会借整个 deref 目标——需要同时
// 借 `durable` 的不同字段（如 `root` 与 `maps`）时，先绑定局部变量（字段级拆借），
// 再调用方法。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::layout::TRAMPOLINE;
use crate::lock::{Level, RelLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::{Frame, FrameState};
use crate::memory::manager::{asid, flush_asid, mode};

use super::SpaceKind;
use super::durable::Durable;
use super::dynamic::Dynamic;
use super::map::{Map, MapKind};
use super::window::{FrameWindow, HeapWindow, StackWindow};

/// 地址空间的可变状态 — 由 [`Space::inner`] 这把 [`RelLock`] 保护。
///
/// 只做组合：`durable`（常数侧）+ 三个窗口（`stack` / `frame` / `heap`）覆盖空间
/// 全部簿记；窗口按种类类型化（`window` 子模块，各窗口自带构造与生命周期操作），
/// `Space` 只经 `with` / `with_flush` 锁一次并编排事务。与页表操作同锁互斥
/// （锁约定见 [`Space`]）。
#[derive(Debug)]
pub(crate) struct SpaceInner {
    /// 常数侧：根页表 + 中间表帧 + 常数映射表。
    pub(crate) durable: Durable,
    /// 任务栈窗口（恒 1 区：顶锚 `[upper() − STACK_WINDOW_SIZE, upper())`）。
    pub(crate) stack: StackWindow,
    /// team 帧区窗口（恒 1 区：内核半区 `[TEAM_FRAME_BASE, +TEAM_FRAME_WINDOW_SIZE)`）。
    pub(crate) frame: FrameWindow,
    /// 用户堆窗口（装载期由 loader 注册一次：`[image_end, 栈底)`；未装载 = `None`）。
    pub(crate) heap: Option<HeapWindow>,
}

impl SpaceInner {
    pub(crate) fn new() -> Result<Self, MapError> {
        // 区间分配器零 up-front（∝ 存活块）；内核空间窗口元数据在基线前稳定由 boot
        // 顺序保证（record_baseline 在 spawn 前）。窗口几何在各窗口类型的 new() 内。
        Ok(Self {
            durable: Durable::new()?,
            stack: StackWindow::new(),
            frame: FrameWindow::new(),
            heap: None,
        })
    }

    /// 全窗口只读视图（resolve / audit / unmap / 重叠检查用）。
    ///
    /// 新窗口种类在此登记一处，其余遍历点（resolve/audit/unmap/overlap）自动覆盖。
    pub(crate) fn windows_ref(&self) -> impl Iterator<Item = &Dynamic> {
        [&self.stack.inner, &self.frame.inner]
            .into_iter()
            .chain(self.heap.iter().map(|h| &h.inner))
    }

    /// 全窗口可变视图（unmap 摘子 Map 用）。
    pub(crate) fn windows_mut(&mut self) -> impl Iterator<Item = &mut Dynamic> {
        [&mut self.stack.inner, &mut self.frame.inner]
            .into_iter()
            .chain(self.heap.iter_mut().map(|h| &mut h.inner))
    }

    /// 把帧注入映射到一段**已登记的动态窗口子 Map**：先在窗口子表按 VA 定位
    /// （各窗口区间互不重叠，按 VA 唯一），再经 [`Durable::map_frames`] 逐帧装 PTE
    /// 并 `inject` 入子 Map（保持「帧 i ↔ va + i·PAGE_SIZE」不变量）。中途失败
    /// 返回错误，调用方 drop Space 统一回收（已装 PTE 与已入帧随空间归还）。
    ///
    /// TLB 刷新由调用方的 [`Space::with_flush`] 负责。
    pub(crate) fn attach_dynamic(
        &mut self,
        va: VirtAddr,
        frames: Vec<Frame>,
    ) -> Result<(), MapError> {
        // 字段级拆借：child 借窗口、durable 另借——同时可用（借用约定见文件头）。
        let SpaceInner {
            durable,
            stack,
            frame,
            heap,
        } = self;
        let child = [&mut stack.inner, &mut frame.inner]
            .into_iter()
            .chain(heap.iter_mut().map(|h| &mut h.inner))
            .flat_map(|w| w.children.iter_mut())
            .find(|m| m.va == va)
            .ok_or(MapError::NoRegion)?;
        let child_flags = child.flags;
        durable.map_frames(va, &frames, child_flags)?;
        for frame in frames {
            child.inject(frame);
        }
        Ok(())
    }

    /// `[start, start+size)` 是否与**常数映射或任何窗口区间**重叠（空间级簿记查询）。
    ///
    /// 边界用 saturating 算术：TRAMPOLINE 等最高页映射的 `va + size` 会溢出 2^64，
    /// 饱和到 `usize::MAX` 即「延伸到地址空间尽头」——比较仍正确。
    pub(crate) fn overlaps(&self, start: VirtAddr, size: usize) -> bool {
        let end = start.as_usize().saturating_add(size);
        self.durable.maps.iter().any(|m| {
            start.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize()
        }) || self.windows_ref().any(|w| {
            start.as_usize() < w.va.as_usize().saturating_add(w.size.get()) && end > w.va.as_usize()
        })
    }

    /// 把调用方配好的物理帧映射到一段连续虚拟地址（**常数侧**装配，经
    /// [`Space::with_flush`] 调用；TLB 刷新由 `with_flush` 负责）。
    ///
    /// 与 [`Space::map`] 的差别只在物理连续性：`map` 假定 `paddr` 起 `size`
    /// **物理连续**（一次装整段），而本方法逐帧装 PTE（[`Durable::map_frames`]）
    /// ——每帧用自身 PA，物理可断，与 [`attach_dynamic`](Self::attach_dynamic) 同范本。
    ///
    /// 场景：程序段装载、用户堆——帧是独立堆分配的 `Box`，物理**不连续**。若只用
    /// `map`，第 i 页 PTE 会被算成 `pa0 + i·PAGE_SIZE`，指向错误的物理页。
    /// 簿记仍登记**一张**多页 [`Map`]：其不变量「帧 i ↔ va + i·PAGE」只约束 VA
    /// 索引、不要求 PA 连续，帧随 Map 所有权回收。
    ///
    /// `vaddr` 必须按页对齐；`frames` 非空，长度即页数（`size = len·PAGE_SIZE`）。
    ///
    /// # Errors
    ///
    /// - `NotAligned` — 帧为空或 vaddr 未页对齐。
    /// - `AlreadyMapped` — 与已有映射/窗口重叠。
    /// - `OutOfMemory` — 页表中间表帧耗尽（不改动空间）。
    pub(crate) fn attach_durable(
        &mut self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
        kind: MapKind,
    ) -> Result<(), MapError> {
        let pages = frames.len();
        if pages == 0 || vaddr.offset() != 0 {
            return Err(MapError::NotAligned);
        }
        let size = pages * PAGE_SIZE;
        if self.overlaps(vaddr, size) {
            return Err(MapError::AlreadyMapped);
        }
        // 逐帧装 PTE（每帧自身 PA，物理可断）+ 登记一张多页常数映射
        let SpaceInner { durable, .. } = self;
        durable.map_frames(vaddr, &frames, flags)?;
        durable.maps.push(Map::new(
            vaddr,
            size,
            flags,
            kind,
            frames.into_iter().map(FrameState::Owned).collect(),
            None,
        ));
        Ok(())
    }

    /// 查询 `vaddr` 所属的映射（常数表 → 动态窗口子表），返回借用（锁内使用，
    /// 与 [`resolve_mut`](Self::resolve_mut) 配对，同 `durable::resolve_ref`）。
    pub(super) fn resolve_ref(&self, vaddr: VirtAddr) -> Option<&Map> {
        if let Some(m) = self.durable.resolve_ref(vaddr) {
            return Some(m);
        }
        for w in self.windows_ref() {
            if w.contains(vaddr)
                && let Some(m) = w.children.iter().rev().find(|m| m.contains(vaddr))
            {
                return Some(m);
            }
        }
        None
    }

    /// 查询 `vaddr` 所属映射的可变引用（缺页注入帧用）。
    pub(super) fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        if let Some(m) = self.durable.resolve_mut(vaddr) {
            return Some(m);
        }
        for w in self.windows_mut() {
            if w.contains(vaddr)
                && let Some(m) = w.children.iter_mut().rev().find(|m| m.contains(vaddr))
            {
                return Some(m);
            }
        }
        None
    }

    /// 登记常数映射（内部版，调用者须持锁；不装 PTE、不注入帧——懒区域用）。
    pub(super) fn declare(
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
        if self.overlaps(start, size) {
            return Err(MapError::AlreadyMapped);
        }
        self.durable
            .maps
            .push(Map::new(start, size, flags, kind, Vec::new(), None));
        Ok(())
    }

    /// 页表读翻译（内部版，调用者须持锁，与 map/unmap 写互斥）。
    pub(super) fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
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

/// 虚拟地址空间（布局随运行模式：用户半区/内核半区几何见 [`mode`]）。
///
/// 拥有根页表与**全部自有物理帧**。用户空间只映射 trampoline 叶 PTE（帧归内核、
/// 不拥有）与自有 trap-context 帧，不复制/共享内核映射。全部可变状态
/// （`SpaceInner`：durable / stack / frame / heap）收进一把 [`RelLock`]；根页表随
/// `durable.root` 持有（Box 自动 drop）。
///
/// 空间种类由 [`kind`](Self::kind) 显式区分（见 [`SpaceKind`]）。
///
/// # Concurrency
///
/// 全部可变状态（`SpaceInner`）收进一把 [`RelLock`]：跨 hart 真自旋互斥、
/// 同 hart 可重入——多核下两个 hart 共享同一空间做 map/unmap/缺页时互斥；同
/// hart 持锁期间同步缺页（异步中断不受 SIE 屏蔽）可重入。**约定**：窗口事务经
/// [`with`](Self::with) / [`with_flush`](Self::with_flush) 锁恰好一次，闭包内不
/// 再调用 `Space` 方法；其余公开方法各自锁恰好一次——任何情况下不得有两个 guard
/// 同时 DerefMut（`&mut` 别名，UB）。
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
/// `durable.root`（页表树，递归归还全部表帧）、`durable.maps` 与窗口的
/// 子 Map 帧随字段自动 drop 归还 frame 池——所有权驱动，无需遍历页表树、
/// 无需手写 deallocate。
#[derive(Debug)]
pub struct Space {
    /// 全部可变状态（durable / 三窗口）——一把可重入锁保护。
    inner: RelLock<SpaceInner>,
    /// 空间种类（内核 / 用户），内嵌 ASID。
    kind: SpaceKind,
}

/// 连续 VA 区间逐段翻译迭代器（`Space::segments` 产出）。
///
/// 每步：经 `Space::translate` 译当前 VA 页 → 产出 `(PA, flags, 页内余量)`。
/// 物理帧可能不连续，故按页切段；flags 随段（消费方据此做 R 权限检查）。
/// 未映射页 → 迭代终止（消费方据完整度处置）。惰性零分配；持锁语义同
/// `translate`（逐页加锁，与 map/unmap 互斥）。
pub struct Segments<'a> {
    space: &'a Space,
    va: usize,
    end: usize,
}

impl Iterator for Segments<'_> {
    type Item = (crate::memory::manager::addr::PhysAddr, PteFlags, usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.va >= self.end {
            return None;
        }
        let (pa, flags) = self
            .space
            .translate(crate::memory::manager::addr::VirtAddr::from_raw(self.va))?;
        let page = self.va & !(PAGE_SIZE - 1);
        let chunk = (page + PAGE_SIZE - self.va).min(self.end - self.va);
        self.va += chunk;
        Some((pa, flags, chunk))
    }
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

    /// 用户空间构造器（独立 ASID，经 [`crate::memory::manager::asid::allocate`] 分配）。
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
    /// 常数映射登记：借用、无帧）。线程 trap 帧在此不创建，按线程另行分配。
    ///
    /// # Errors
    ///
    /// 页表帧耗尽时返回 [`MapError::OutOfMemory`]。
    fn seed_user(&self, space: &mut Space) -> Result<(), MapError> {
        // 读内核空间的 trampoline 叶 PTE（只读）
        let (tramp_pa, tramp_flags) = {
            let ks_inner = crate::work::unit::team::kernel()
                .expect("kernel team not initialized")
                .space
                .inner
                .lock();
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

impl Space {
    // ── 身份 ──────────────────────────────────────────────────

    /// 空间种类（内核 / 用户）——任务模式判定的单一事实源
    /// （调度域经 `team.space.kind()` 区分内核/用户任务路径）。
    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    // ── 事务入口 ──────────────────────────────────────────────

    /// 簿记事务：锁恰好一次，闭包内直接操作 [`SpaceInner`]（窗口簿记：allocator /
    /// children / windows，不改页表、不装 PTE）；锁外不刷 TLB。
    ///
    /// 纪律：闭包内**不得再调用 `Space` 的任何方法**——会重入取第二个 guard，
    /// 两个 guard 同时 DerefMut 构成 `&mut` 别名（UB）。
    pub(crate) fn with<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let mut inner = self.inner.lock();
        let r = op(&mut inner);
        drop(inner);
        r
    }

    /// 写页表事务：同 [`with`](Self::with)，锁外按本空间 ASID 刷 TLB——PTE 变更
    /// 跨核可见（只失效本地址空间旧条目，其它任务 TLB 保留）。
    ///
    /// # Safety
    ///
    /// TLB 刷新由 `flush_asid` 负责（见 `flush_asid` 的 Safety 段）。
    pub(crate) fn with_flush<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let r = self.with(op);
        // SAFETY: sfence.vma 见 flush_asid
        unsafe {
            flush_asid(self.kind.asid());
        }
        r
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
        if size == 0 || vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        if inner.overlaps(vaddr, size) {
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
        // SAFETY: sfence.vma 见 flush_asid
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
        // 页表侧：清叶 PTE + 回收变空的中间表
        inner.durable.unmap_frames(vaddr, size);
        // 簿记侧：移除被完全覆盖的常数映射与窗口子 Map
        let covered = |m: &Map| {
            vaddr.as_usize() <= m.va.as_usize()
                && end >= m.va.as_usize().saturating_add(m.size.get())
        };
        inner.durable.maps.retain(|m| !covered(m));
        for w in inner.windows_mut() {
            w.children.retain(|m| !covered(m));
        }
        drop(inner);

        // SAFETY: sfence.vma 见 flush_asid
        unsafe {
            flush_asid(self.kind.asid());
        }
    }

    /// 修改已映射区域的保护标志（mprotect）。
    ///
    /// 懒区感知、按映射种类分流：
    /// - **懒 Anonymous 映射**（mmap/declare/堆）：逐页经 [`Map::translate`] 判
    ///   触态——已触页（帧在册）当场翻叶子 PTE；未触页（无帧无 PTE）只同步簿记
    ///   `Map.flags`，后续缺页按新标志物化。
    /// - **非懒映射**（即时/借用，如内核 .rodata 恒等区）：叶子本应在册，直接
    ///   `walk_mut` 翻位；叶子缺失即 [`MapError::NotMapped`]。
    ///
    /// `V` 恒被强制置位（保护标志只表达 R/W/X/U 语义）；子区间 mprotect 整张
    /// 覆盖 Map 的 `flags` 平铺（教学简化）。
    ///
    /// # Errors
    ///
    /// - 区间任一页不在任何映射内 → [`MapError::NoRegion`]；
    /// - 非懒映射缺叶子 → [`MapError::NotMapped`]。
    pub fn mprotect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let flags = flags | PteFlags::V;
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let (kind, eager_or_touched) = {
                let m = inner.resolve_ref(va).ok_or(MapError::NoRegion)?;
                if m.kind == MapKind::Anonymous {
                    // translate = 按 VA→PA 语义直接取帧：命中 = 懒区已触页（帧在册）
                    (m.kind, m.translate(va.as_usize() - m.va.as_usize()).is_ok())
                } else {
                    // 非懒映射：叶子应已在册，直接走 walk_mut（缺失即 NotMapped）
                    (m.kind, true)
                }
            };
            if eager_or_touched {
                // 已触页/即时映射必有叶子：不分配中间表（None），缺失即错误
                let leaf = inner.durable.root.walk_mut(va, false, mode::levels())?;
                leaf.set_flags(flags);
            }
            if kind == MapKind::Anonymous {
                inner
                    .resolve_mut(va)
                    .expect("map exists (checked above)")
                    .flags = flags;
            }
        }
        drop(inner);
        // SAFETY: sfence.vma 见 flush_asid
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
                let leaf = durable.root.walk_mut(va, false, mode::levels())?;
                let ppn = (arc_pa.as_usize() >> 12) as u64;
                leaf.set(
                    ppn,
                    (flags & !PteFlags::W) | PteFlags::A | PteFlags::D | PteFlags::V,
                );
            }
        }
        drop(inner);
        // SAFETY: sfence.vma 见 flush_asid
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
    #[allow(clippy::wrong_self_convention)] // Space 跨核 Arc 共享，&self 刻意为之：可变性全在 inner 锁内
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
                let leaf = durable.root.walk_mut(page, false, mode::levels())?;
                leaf.set_flags(leaf.flags() | PteFlags::W | PteFlags::V);
                return Ok(());
            }
            let new_box: Frame = {
                let FrameState::Borrowed(arc) = &map.frames[idx] else {
                    unreachable!("checked above")
                };
                let mut nb: crate::memory::manager::table::Frame = unsafe {
                    Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                };
                nb.copy_from_slice(&arc[..]);
                nb
            };
            let pa = PhysAddr::from_raw(new_box.as_ptr() as usize);
            let old = core::mem::replace(&mut map.frames[idx], FrameState::Owned(new_box));
            drop(old); // 放下共享 Arc（计数 −1）
            let leaf = durable.root.walk_mut(page, false, mode::levels())?;
            let ppn = (pa.as_usize() >> 12) as u64;
            leaf.set(ppn, flags | PteFlags::W | PteFlags::V);
        }
        drop(inner);
        // SAFETY: sfence.vma 见 flush_asid
        unsafe {
            flush_asid(self.kind.asid());
        }
        Ok(())
    }

    /// 无条件私有化：立即分裂成私有 Owned（fork 后父方脱离共享用）。
    /// 语义 = to_mut（保证该页从此私有可写）。
    #[allow(dead_code)] // fork 后端预留
    #[allow(clippy::wrong_self_convention)] // 同 to_mut：Arc 共享，&self 刻意为之
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
    /// （unmap / Space::drop）drop 整个 Map 时随帧自动释放，
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
    pub fn page_fault(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        let mut inner = self.inner.lock();
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            // 前提：地址已登记 Anonymous 映射；flags 由映射自取（含 A/D）。
            let flags = {
                let m = inner.resolve_ref(va).ok_or(MapError::NoRegion)?;
                if m.kind != MapKind::Anonymous {
                    return Err(MapError::NoRegion);
                }
                m.flags | PteFlags::A | PteFlags::D
            };
            let page: crate::memory::manager::table::Frame = unsafe {
                Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                    .map_err(|_| MapError::OutOfMemory)?
                    .assume_init()
            };
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            {
                let SpaceInner { durable, .. } = &mut *inner;
                durable.root.map(va, pa, PAGE_SIZE, flags)?;
            }
            let map = inner.resolve_mut(va).expect("map exists (checked above)");
            map.inject(page);
        }
        drop(inner);
        // SAFETY: sfence.vma 见 flush_asid
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
        for w in inner.windows_ref() {
            w.children.iter().for_each(check);
        }
    }

    // ── 查询 ──────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位（页表读路径）。
    ///
    /// 未映射时返回 `None`。持锁与 map/unmap 的页表写互斥。
    /// **单点查询原语**：缺页判定 / 映射存在性断言用；连续区间遍历见 [`segments`]。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        let inner = self.inner.lock();
        inner.translate(vaddr)
    }

    /// 连续 VA 区间 → 逐段物理翻译（[`Segments`] 迭代器）。
    ///
    /// 逐页步进（每页经 [`translate`] 翻译，物理帧可能不连续——VA 连续 ≠ PA
    /// 连续），产出 `(物理地址, 段字节数)`；某页未映射即停（调用方据不完整
    /// 结果处置）。零分配、持锁语义同 `translate`（逐页加锁）。
    ///
    /// 消费方：「连续 VA → 多段可直读 PA」的唯一遍历原语。
    pub fn segments(&self, va: VirtAddr, len: usize) -> Segments<'_> {
        Segments {
            space: self,
            va: va.as_usize(),
            end: va.as_usize() + len,
        }
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
        self.inner.lock().declare(start, size, flags, kind)
    }

    /// 查询虚拟地址所属映射的**种类**（常数表 → 动态窗口子表）。
    ///
    /// 缺页分支用：Anonymous → 走懒分配；Reserved → 预留诊断；None → 无映射。
    pub fn resolve_kind(&self, vaddr: VirtAddr) -> Option<MapKind> {
        self.inner.lock().resolve_ref(vaddr).map(|m| m.kind)
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        // 先释放本空间的 ASID：释放内部会 sfence 该 ASID 的 TLB 残留条目（ASID
        // 可能被后续任务复用，旧条目须失效）。内核空间 ASID 0 不参与分配。
        if let SpaceKind::User { asid } = self.kind {
            asid::deallocate(asid);
        }
        // `inner` 随字段自动 drop：root（页表树，递归归还全部表帧）/maps 与窗口子 Map 的帧全部归还
        // frame 池——所有权驱动，无遍历页表树、无手写 deallocate。
    }
}
