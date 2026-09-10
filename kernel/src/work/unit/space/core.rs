// core — 主类型 [`Space`] / [`SpaceBuilder`] / [`SpaceInner`] + 映射原语
//
// # 层职责（核心/适配分离）
//
// - [`SpaceInner`] = 核心：数据 + 全部操作体，**无锁无刷**（只出现在事务闭包内）。
// - [`Space`] = 适配：`RelLock` 门（[`with`]/[`with_flush`]）+ 每操作 ≤3 行转发；
//   锁外按本空间 ASID 刷 TLB。`Space` 不知道栈/帧/堆/mmap 是什么。
//
// # 锁约定（`Space::inner`，RelLock）
//
// 全部可变状态由 `RelLock` 互斥（跨 hart 真自旋，同 hart 可重入）。`kind`
// 分配后不可变，故 [`Space`] 可放心跨线程传递。
//
// 锁约定：窗口事务统一经 [`Space::with`] / [`Space::with_flush`]（锁恰好一次，
// 闭包内直接操作 `inner`）；其余公开方法各自锁恰好一次、不重入。`with` 闭包内
// **不得再调用** `Space` 的任何方法。
//
// 页表树读写与 `SpaceInner` 数据共享同一把锁。**段表并入 Space 锁**：`Segment`
// 无锁，全部 `allocate`/`deallocate` 必须发生在 `with`/`with_flush` 事务内——
// 任何绕开 `Space::with` 的取段/还段都是破坏锁序（死锁/数据竞争）。
//
// 借用约定：guard 是 `Deref`，方法调用的自动引用会借整个 deref 目标——需要
// 同时借 `durable` 的不同字段时，先绑定局部变量（字段级拆借），再调用方法。

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::mode;
use crate::memory::manager::table::{Frame, FrameState, TableNode};

use super::Seg;
use super::map::{Map, Pending};
use super::salvage::Salvage;

/// 地址空间的可变状态 — 由 [`Space::inner`] 这把 [`RelLock`] 保护。
///
/// 纯映射簿记：页表树 + 两段 + 唯一映射表。`user` 段在装载/内核 init 前未
/// 就绪（`None`），经 [`Self::dynamic`] 设置一次；`kernel` 段为布局常量域
/// 构造即定。全部操作体都在这里（无锁无刷）：`Space` 只转发。
pub(crate) struct SpaceInner {
    /// 页表树（翻译基础，全空间一棵）。
    pub(crate) root: TableNode,
    /// 用户半区段 `[free_base, upper)` — 栈/堆/dock 同池（dynamic 前 None）。
    pub(crate) user: Option<super::seg::Segment>,
    /// 内核 trap 帧常量区段 `[TEAM_FRAME_BASE, +SIZE)`，S-only。
    pub(crate) kernel: super::seg::Segment,
    /// 唯一 VA→PA 簿记（单表遍历，无常数/动态之分）。
    pub(crate) maps: Vec<Map>,
}

impl core::fmt::Debug for SpaceInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpaceInner")
            .field("user_attached", &self.user.is_some())
            .field("maps", &self.maps.len())
            .finish()
    }
}

impl SpaceInner {
    // ── 构造 / 就位 ─────────────────────────────────────────

    /// 常量侧就位：根页表 + kernel 段（布局常量域构造即定）。
    pub(crate) fn durable() -> Result<Self, MapError> {
        Ok(Self {
            root: TableNode::root()?,
            user: None,
            kernel: super::seg::Segment::new(
                TEAM_FRAME_BASE.as_usize(),
                TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE,
            ),
            maps: Vec::new(),
        })
    }

    /// 运行期侧就位：user 段边界 `[base, upper)`（恰好一次；任何 user 段分配
    /// 之前）。`base` 通常 = 映像装载结束地址（loader）或内核镜像基址。
    ///
    /// # Panics
    ///
    /// 重复设置（user 段已在册）或 base 非页对齐 / 越过 upper。
    pub(crate) fn dynamic(&mut self, base: usize) {
        assert!(self.user.is_none(), "Space: user segment double attach");
        let edge = mode::upper().as_usize();
        assert!(
            base.is_multiple_of(PAGE_SIZE) && base <= edge,
            "Space: bad user segment [{base:#x}, {edge:#x})"
        );
        self.user = Some(super::seg::Segment::new(base, edge));
    }

    // ── 段轴 ────────────────────────────────────────────────

    /// 从段取一块 VA（lowest first-fit）。
    ///
    /// # Errors
    ///
    /// 段未就绪（user 未 dynamic）→ [`MapError::NoRegion`]；段空隙不足 →
    /// [`MapError::OutOfMemory`]。
    pub(crate) fn allocate(&mut self, seg: Seg, size: usize) -> Result<VirtAddr, MapError> {
        let base = match seg {
            Seg::User => self
                .user
                .as_mut()
                .ok_or(MapError::NoRegion)?
                .allocate(size)
                .map_err(|_| MapError::OutOfMemory)?,
            Seg::Kernel => self
                .kernel
                .allocate(size)
                .map_err(|_| MapError::OutOfMemory)?,
        };
        Ok(VirtAddr::from_raw(base))
    }

    /// 还段：精确匹配释放 `(addr, size)`。未分配 / 长度不匹配 → `false`。
    pub(crate) fn deallocate(&mut self, seg: Seg, addr: usize, size: usize) -> bool {
        let seg = match seg {
            Seg::User => match self.user.as_mut() {
                Some(u) => u,
                None => return false,
            },
            Seg::Kernel => &mut self.kernel,
        };
        seg.deallocate(addr, size)
    }

    // ── 装配族 ──────────────────────────────────────────────

    /// 只登记簿记：校验 + 推入一张空帧 Map（懒/守卫/借用占位），不装 PTE。
    ///
    /// `pending: Some(Lazy)` = 缺页物化；`Some(Guard)` = 禁止触碰；
    /// `None` + 空帧 = 借用占位（leaf 由调用方另装，见 [`Self::borrow`]）。
    pub(crate) fn map(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Result<(), MapError> {
        if size == 0 || !va.as_usize().is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE)
        {
            return Err(MapError::NotAligned);
        }
        if self.overlaps(va, size) {
            return Err(MapError::AlreadyMapped);
        }
        self.maps
            .push(Map::new(va, size, flags, pending, BTreeMap::new()));
        Ok(())
    }

    /// 立即装配（Eager）：登记全物化 Map + 逐页取帧、装叶、注入（物理可断）。
    ///
    /// **帧来源由调用者给**（与 [`Self::attach`] 同形）：本动作服务三种对象
    /// （trap 帧 / 用户堆页 / 任务栈），是"造哪种对象"的那一层说出种类——
    /// `|| Ok(crate::tag!(Trap, SpaceInner::frame()?))`。本函数与 [`Self::frame`]
    /// 都不认识种类。
    ///
    /// 中途帧耗尽回滚**装配**（清已装叶 + 摘自身 Map）。
    pub(crate) fn claim<F>(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        next_frame: F,
    ) -> Result<(), MapError>
    where
        F: FnMut() -> Result<Frame, MapError>,
    {
        if size == 0 || !va.as_usize().is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        if self.overlaps(va, size) {
            return Err(MapError::AlreadyMapped);
        }
        let pages = size / PAGE_SIZE;
        // 先登记空 map（全物化 pending None），再走 install 装帧
        self.maps
            .push(Map::new(va, size, flags, None, BTreeMap::new()));
        self.install(va, pages, flags, MapMode::Claim(va), next_frame)
    }

    /// 装配调用方配好的帧（物理可断，逐帧装叶）+ 登记全物化 Map。
    /// 帧随 Map drop 归还。`frames` 非空；失败回滚清已装叶。
    pub(crate) fn attach(
        &mut self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        let pages = frames.len();
        if pages == 0 || vaddr.offset() != 0 {
            return Err(MapError::NotAligned);
        }
        let size = pages * PAGE_SIZE;
        if self.overlaps(vaddr, size) {
            return Err(MapError::AlreadyMapped);
        }
        // 先登记空 map（全物化 pending None），再走 install 用外部帧装叶
        self.maps
            .push(Map::new(vaddr, size, flags, None, BTreeMap::new()));
        let mut iter = frames.into_iter();
        self.install(vaddr, pages, flags, MapMode::Claim(vaddr), move || {
            Ok(iter.next().expect("attach: frame iter exhausted"))
        })
    }

    /// 借帧连续映射：物理地址已知、一次装连续段，不持帧（帧归外部：机器/
    /// 内核/DockMeta）。DRAM 恒等、trampoline、dock/ring 视图走这。
    pub(crate) fn borrow(
        &mut self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0 || vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        if self.overlaps(vaddr, size) {
            return Err(MapError::AlreadyMapped);
        }
        self.root.map(vaddr, paddr, size, flags)?;
        self.maps
            .push(Map::new(vaddr, size, flags, None, BTreeMap::new()));
        Ok(())
    }

    // ── 拆除 ────────────────────────────────────────────────

    /// 统一拆除 `[va, va+size)`：逐 map 清其相交且**真有 PTE** 的页（中间表随之
    /// 回收）→ **全覆盖**的 Map 整张摘除、**部分覆盖**的 Map 按洞分裂——摘下的帧
    /// 一律**交料箱**（`salvage`），清退到齐后才归还（远核可能仍持旧条目）。不碰段。
    pub(crate) fn unmap(&mut self, va: VirtAddr, size: usize, salvage: &mut Salvage) {
        if size == 0 {
            return;
        }
        let end = va.as_usize().saturating_add(size);
        let mut survivors: Vec<Map> = Vec::new();
        for mut m in core::mem::take(&mut self.maps) {
            let s = m.va.as_usize();
            let m_end = s.saturating_add(m.size.get());
            let lo = va.as_usize().max(s);
            let hi = end.min(m_end);
            if lo >= hi {
                survivors.push(m); // 不相交
                continue;
            }
            let lo_pg = (lo - s) / PAGE_SIZE;
            let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
            // 清叶与摘 map 同一趟：拿着 map 才能问它哪些页有 PTE，顺序不可能写反。
            let root = &mut self.root;
            m.runs(lo_pg, hi_pg, |rva, rsize| root.unmap(rva, rsize));
            if va.as_usize() <= s && end >= m_end {
                salvage.take_map(m); // 全覆盖：整张摘除交料箱
                continue;
            }
            // 部分覆盖：挖洞分裂（洞内帧由 carve 交料箱）
            let right = m.carve(lo_pg, hi_pg, salvage);
            survivors.push(m); // 左段（carve 收缩 self）
            if let Some(right) = right {
                survivors.push(right);
            }
        }
        self.maps = survivors;
    }

    /// 只读校验：`(addr, size)` 是否为该段的一个已分配块（拆除路径的失败域
    /// 前移，见 [`super::seg::Segment::holds`]）。
    pub(crate) fn holds(&self, seg: Seg, addr: usize, size: usize) -> bool {
        match seg {
            Seg::User => self.user.as_ref().is_some_and(|u| u.holds(addr, size)),
            Seg::Kernel => self.kernel.holds(addr, size),
        }
    }

    // ── 帧 ──────────────────────────────────────────────────

    /// 领一帧（零化）——全模块唯一帧分配点，**不认识种类**。
    ///
    /// 种类由造对象的那一层标注：懒页在本文件的 [`Self::materialize`]，trap 帧 /
    /// 堆页 / 栈在各自 window 的装配请求里（帧来源以闭包给出），COW 在本文件的
    /// 分裂路径。帧分配器与装配核心都不携带种类参数。
    pub(crate) fn frame() -> Result<Frame, MapError> {
        let frame: Frame = unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        };
        Ok(frame)
    }

    // ── 物化 / 保护 / 共享 ──────────────────────────────────

    /// 懒页物化：查 Lazy 映射 → 分配零页 → 装叶 + 注入帧。
    ///
    /// `pending` 非 Lazy（Guard / None）或无映射 → 错误。
    /// 循环失败时 [`InstallGuard`] 按 [`MapMode::Materialize`] 自动拆 PTE +
    /// 摘 frames 键；收尾成功须 `commit()` 拆雷。
    pub(crate) fn materialize(&mut self, va: VirtAddr, size: usize) -> Result<(), MapError> {
        let pages = size.div_ceil(PAGE_SIZE);
        // 前置：从已存在的 Lazy map 拿 flags + 校验 pending
        let flags = {
            let m = self.resolve_ref(va).ok_or(MapError::NoRegion)?;
            if m.pending != Some(Pending::Lazy) {
                return Err(MapError::NoRegion);
            }
            m.flags | PteFlags::A | PteFlags::D
        };
        // 种类 = Lazy：本动作只服务懒页一种对象，故帧来源就地给出。
        self.install(va, pages, flags, MapMode::Materialize, || {
            Ok(crate::tag!(Lazy, Self::frame()?))
        })
    }

    /// 修改已映射区域的保护标志：逐 map 分流——**真有 PTE** 的页翻叶 PTE（借用/
    /// 全物化整段、懒区只走触过的页，见 [`Self::runs`]）；懒区另同步 `map.flags`
    /// （未触页将来物化时按新标志装）；guard 只同步 flags。
    ///
    /// 先校验覆盖、后落改（`maps` 互不重叠 ⇒ 相交长度之和 == size ⟺ 全覆盖）。
    ///
    /// # Errors
    ///
    /// - `NoRegion` — 区间内有页不落在任何 map（此时尚未落任何改动）
    /// - 叶操作失败 — 簿记与页表分叉（不变量违反，见 `audit` 的反向核对）
    pub(crate) fn protect(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0 {
            return Ok(());
        }
        let flags = flags | PteFlags::V;
        let end = va.as_usize().saturating_add(size);
        let span = |m: &Map| {
            let s = m.va.as_usize();
            let lo = va.as_usize().max(s);
            let hi = end.min(s.saturating_add(m.size.get()));
            (lo < hi).then_some((s, lo, hi))
        };
        // 1. 覆盖校验先行：不足即整体失败，状态未动。
        let covered: usize = self
            .maps
            .iter()
            .filter_map(&span)
            .map(|(_, lo, hi)| hi - lo)
            .sum();
        if covered != size {
            return Err(MapError::NoRegion);
        }
        // 2. 落改（root 与 maps 是不同字段，可同时可变借出）
        let mut fault: Option<MapError> = None;
        let root = &mut self.root;
        for m in self.maps.iter_mut() {
            let Some((s, lo, hi)) = span(m) else { continue };
            let lo_pg = (lo - s) / PAGE_SIZE;
            let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
            m.runs(lo_pg, hi_pg, |rva, rsize| {
                if fault.is_none()
                    && let Err(e) = root.protect(rva, rsize, flags)
                {
                    fault = Some(e);
                }
            });
            if m.pending == Some(Pending::Lazy) {
                m.flags = flags;
            }
        }
        match fault {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 把 `[start, start+size)` 内可写页提升为共享只读：Owned → Shared(Arc)。
    /// 写缺页将触发 [`Self::own`] 分裂。
    ///
    /// **Lazy 区行为**：未触页（`frames` 无键）跳过——首次写缺页命中 Owned
    /// 而非 Shared，触发 [`Self::own`] 从 Owned 直接分裂（不走 COW 路径）。
    /// 这是 fork 后端语义，不为 bug。
    #[allow(dead_code)] // fork 后端预留
    pub(crate) fn share(&mut self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        let pages = size.div_ceil(PAGE_SIZE);
        let mut guard = InstallGuard::new(self, start, MapMode::Materialize);
        let result: Result<(), MapError> = (|| {
            for i in 0..pages {
                let va = start + i * PAGE_SIZE;
                // 拿原 Owned 字节 + flags + idx
                let (bytes_src, flags, idx) = {
                    let map = guard.inner.resolve_ref(va).ok_or(MapError::NotMapped)?;
                    let idx = (va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                    match &map.frames.get(&idx) {
                        // 跳过的页不 mark——保留原 Shared/None 状态
                        Some(FrameState::Shared(_)) => continue,
                        Some(FrameState::Owned(b)) => (b.as_slice(), map.flags, idx),
                        None => continue,
                    }
                };
                // 种类 = Cow：COW 共享帧（Shared）——关机归零。
                let mut arc: Arc<[u8; PAGE_SIZE], &'static dyn alloc::alloc::Allocator> = crate::tag!(
                    Cow,
                    Arc::new_in(
                        [0u8; PAGE_SIZE],
                        crate::memory::allocator::frame::allocator()
                    )
                );
                Arc::get_mut(&mut arc)
                    .expect("fresh arc")
                    .copy_from_slice(bytes_src);
                let arc_pa = PhysAddr::from_raw(Arc::as_ptr(&arc) as usize);
                {
                    let map = guard.inner.resolve_mut(va).ok_or(MapError::NotMapped)?;
                    let old = map.frames.insert(idx, FrameState::Shared(arc));
                    drop(old); // 原 Owned 帧归还 frame 池
                    let leaf = guard.inner.root.walk_mut(va, false, mode::levels())?;
                    let ppn = (arc_pa.as_usize() >> 12) as u64;
                    leaf.set(
                        ppn,
                        (flags & !PteFlags::W) | PteFlags::A | PteFlags::D | PteFlags::V,
                    );
                }
                guard.mark(i); // 已 set PTE：登记回滚页号（跳过的页不登记）
            }
            Ok(())
        })();
        if result.is_ok() {
            guard.commit();
        }
        result
    }

    /// COW 写缺页分裂：保证 `[start, start+size)` 内每页私有可写。
    /// Shared → 分新 Owned 拷字节；Owned + 只读 → 翻 W；其它页**静默跳过**
    /// （无 map / 帧未触——与 [`Self::share`] 跳过非 Owned 对称）。
    #[allow(clippy::wrong_self_convention)] // Space 跨核 Arc 共享，&mut self 在事务内
    pub(crate) fn own(&mut self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        enum Step {
            /// Owned 页 PTE 翻 W（已 Owned + 只读）
            SetW,
            /// Shared 页分裂：新 Owned 帧 + 写可 PTE
            Split(PteFlags),
        }
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let page = start + i * PAGE_SIZE;
            // 1. 探：决定本页动作（静默跳过 no-map / 帧未触）
            let step: Step = match self.resolve_mut(page) {
                Some(map) => {
                    let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                    let flags = map.flags;
                    match map.frames.get(&idx) {
                        Some(FrameState::Owned(_)) => Step::SetW,
                        Some(FrameState::Shared(_)) => Step::Split(flags),
                        None => continue,
                    }
                }
                None => continue,
            };
            // 2. 行
            match step {
                Step::SetW => {
                    let leaf = self.root.walk_mut(page, false, mode::levels())?;
                    leaf.set_flags(leaf.flags() | PteFlags::W | PteFlags::V);
                }
                Step::Split(flags) => {
                    let arc = {
                        let map = self.resolve_mut(page).expect("map exists (checked)");
                        let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                        match &map.frames.get(&idx) {
                            Some(FrameState::Shared(a)) => a.clone(),
                            _ => continue, // 并发下变 Owned——跳过
                        }
                    };
                    // 种类 = Cow：COW 分裂出的新帧——关机归零。
                    let mut nb: Frame = crate::tag!(Cow, Self::frame()?);
                    nb.copy_from_slice(&arc[..]);
                    let ppn = (PhysAddr::from_raw(nb.as_ptr() as usize).as_usize() >> 12) as u64;
                    let map = self.resolve_mut(page).expect("map exists (checked)");
                    let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                    let old = map.frames.insert(idx, FrameState::Owned(nb));
                    drop(old);
                    let leaf = self.root.walk_mut(page, false, mode::levels())?;
                    leaf.set(ppn, flags | PteFlags::W | PteFlags::V);
                }
            }
        }
        Ok(())
    }

    // ── 查询 ────────────────────────────────────────────────

    /// `[start, start+size)` 是否与**已有映射**重叠（单表查询）。
    pub(crate) fn overlaps(&self, start: VirtAddr, size: usize) -> bool {
        let end = start.as_usize().saturating_add(size);
        self.maps.iter().any(|m| {
            start.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize()
        })
    }

    /// 查询 `vaddr` 所属的映射（常数表 → 动态窗口子表），返回借用。
    pub(super) fn resolve_ref(&self, vaddr: VirtAddr) -> Option<&Map> {
        self.maps.iter().rev().find(|m| m.contains(vaddr))
    }

    /// 查询 `vaddr` 所属映射的可变引用（缺页注入帧用）。
    pub(super) fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        self.maps.iter_mut().rev().find(|m| m.contains(vaddr))
    }

    /// 页表读翻译（内部版，调用者须持锁）。
    pub(super) fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.root
            .walk_ref(vaddr)
            .map(|x| (x.0 + vaddr.offset(), x.1))
            .ok()
    }

    /// 簿记↔页表**双向**一致性核对（boot / 压力测试后调用；不一致即 panic）。
    ///
    /// 正向（map ⇒ PTE）：每条簿记帧都能在页表里翻到同一物理页。
    /// 反向（PTE ⇒ map）：每个已装叶都落在某张 map 内、且该页确实"已物化"——
    /// 拆除与改权只碰**簿记说有 PTE** 的页（[`Self::runs`]），这条一破，PTE 就会
    /// 残留成悬垂（指向已归还帧，复用即错乱）。
    #[cfg(feature = "audit")]
    pub(crate) fn audit(&self) {
        for m in &self.maps {
            for (i, f) in &m.frames {
                let va = m.va + i * PAGE_SIZE;
                let expect = f.pa();
                match self.translate(va) {
                    Some((pa, _)) if pa == expect => {}
                    other => panic!(
                        "space audit @{:#x}: pte {other:?} != frame {expect:#x} (map {:#x}+{})",
                        va.as_usize(),
                        m.va.as_usize(),
                        m.size.get()
                    ),
                }
            }
        }
        self.root
            .mapped(mode::levels() - 1, 0, &mut |va: VirtAddr| {
                let Some(m) = self.resolve_ref(va) else {
                    panic!(
                        "space audit @{:#x}: leaf pte outside every map",
                        va.as_usize()
                    );
                };
                let idx = (va.as_usize() - m.va.as_usize()) / PAGE_SIZE;
                assert!(
                    m.is_materialized(idx),
                    "space audit @{:#x}: leaf pte on unmaterialized page (map {:#x}+{}, {:?})",
                    va.as_usize(),
                    m.va.as_usize(),
                    m.size.get(),
                    m.pending
                );
            });
    }
}

// ── 安装回滚 ────────────────────────────────────────────

/// 失败时 maps 簿记的两种处置（由"map 是谁 push 的"决定）。
#[derive(Clone, Copy)]
enum MapMode {
    /// map 由外部预存（materialize 的 Lazy 区——reserve 时已 push）
    /// 失败：保留 map，只摘 frames 键
    Materialize,
    /// map 由当前函数 push（claim / attach——循环前 push 空 map）
    /// 失败：按 va 摘整张
    Claim(VirtAddr),
}

/// 安装回滚守卫——循环失败时按已装页数清叶 + 按 [`MapMode`] 处置 maps。
///
/// `installed: usize` = 已装页数；`commit()` 归零 → drop 循环 0 次 = no-op。
/// 成功路径必须 `commit()`，否则 drop 会拆掉刚装的页。
struct InstallGuard<'a> {
    inner: &'a mut SpaceInner,
    va: VirtAddr,
    installed: BTreeSet<usize>,
    book: MapMode,
}

impl<'a> InstallGuard<'a> {
    fn new(inner: &'a mut SpaceInner, va: VirtAddr, book: MapMode) -> Self {
        Self {
            inner,
            va,
            installed: BTreeSet::new(),
            book,
        }
    }
    fn mark(&mut self, at: usize) {
        self.installed.insert(at);
    }
    /// 拆雷：drop 时 installed=0 不调 unmap；book 不动——Claim 仍按原策略摘整张 map。
    /// 消费 self。
    fn commit(mut self) {
        self.installed.clear();
    }
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        // commit() 已归零 → 成功路径，整个 drop 不做任何动作
        if self.installed.is_empty() {
            return;
        }
        // 1. 按精确页号清已 set 叶 PTE（单页 unmap，各自拆空中间表）
        for &j in &self.installed {
            self.inner.root.unmap(self.va + j * PAGE_SIZE, PAGE_SIZE);
        }
        // 2. 按策略动 maps（精确页号——share 跳过的页保留原状态）
        match self.book {
            MapMode::Materialize => {
                for &j in &self.installed {
                    if let Some(m) = self.inner.resolve_mut(self.va + j * PAGE_SIZE) {
                        m.frames.remove(&j);
                    }
                }
            }
            MapMode::Claim(va) => {
                self.inner.maps.retain(|m| m.va != va);
            }
        }
    }
}

impl SpaceInner {
    /// 装配 N 页——materialize / claim / attach 三动作的统一核心。
    ///
    /// 前置：调用方已 push 一张覆盖 `[va, va+pages*PAGE_SIZE)` 的 map：
    /// - materialize：Lazy 区（reserve 时已 push）
    /// - claim / attach：本函数循环前 push 空 map
    ///
    /// `next_frame` 按页提供帧；`flags` 直接装入 PTE。
    /// 失败时 [`InstallGuard`] 按 `book` 处置：清叶 + 摘 frames 键或摘整张 map。
    fn install<F>(
        &mut self,
        va: VirtAddr,
        pages: usize,
        flags: PteFlags,
        book: MapMode,
        mut next_frame: F,
    ) -> Result<(), MapError>
    where
        F: FnMut() -> Result<Frame, MapError>,
    {
        let mut guard = InstallGuard::new(self, va, book);
        let result: Result<(), MapError> = (|| {
            for i in 0..pages {
                let m_va = va + i * PAGE_SIZE;
                let page = next_frame()?;
                let pa = PhysAddr::from_raw(page.as_ptr() as usize);
                guard.inner.root.map(m_va, pa, PAGE_SIZE, flags)?;
                let map = guard.inner.resolve_mut(m_va).expect("map exists");
                let idx = (m_va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                map.inject(idx, page);
                guard.mark(i);
            }
            Ok(())
        })();
        if result.is_ok() {
            guard.commit();
        }
        result
    }
}
