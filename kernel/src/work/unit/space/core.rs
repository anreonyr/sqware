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
use core::num::NonZeroUsize;

use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE, TRAMPOLINE};
use crate::lock::{Level, RelLock};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::evict::{self, Deaf};
use crate::memory::manager::table::{Frame, FrameState, TableNode};
use crate::memory::manager::{asid, flush_asid, mode};

use super::map::{Map, Pending, PendingState};
use super::{Seg, SpaceKind};

/// 一段 VA 区间 — 分配/映射动作的产物 = 回收的输入（类型同一）。
///
/// 由窗口 claim/allocate/mmap 产出，经 [`Space::release`] 回收。`pa` 仅对
/// **已物化固定帧**有意义（trap 帧恒 Some，栈/懒区恒 None）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    /// 所属段（回收定位）。
    pub(crate) seg: Seg,
    /// 基址（页对齐）。
    pub(crate) va: VirtAddr,
    /// 总长（页对齐，非零）。
    pub(crate) size: NonZeroUsize,
    /// 物化帧物理地址（trap 帧恒 Some；栈/懒区恒 None）。
    pub(crate) pa: Option<PhysAddr>,
}

impl Span {
    pub(crate) fn new(seg: Seg, va: VirtAddr, size: usize, pa: Option<PhysAddr>) -> Self {
        Self {
            seg,
            va,
            size: NonZeroUsize::new(size).expect("span size must be non-zero"),
            pa,
        }
    }
}

/// 拆除产出的待回收料 —— 摘下的帧（整张 [`Map`]）与待还的段（[`Span`]）。
///
/// 硬不变量：**清退到齐之前不得易主**。帧归还即可被别的空间拿到、还段即 VA 可
/// 被本空间复用，而远核此刻可能仍持旧 TLB 条目 —— 两者都必须等在
/// [`Self::reclaim`] 里的清退之后。故 `Drop` 断言料箱已空（非空即被绕过）。
#[must_use = "salvage holds frames/segments that must be reclaimed after eviction"]
pub(crate) struct Salvage {
    /// 摘下的映射（帧随其 drop 归还 frame 池 / Arc 计数归零）。
    maps: Vec<Map>,
    /// 待还的段区间（`release` 类拆除；`unmap` 类不还段则为空）。
    spans: Vec<Span>,
}

impl Salvage {
    /// 空料箱（拆除事务前构造，事务内收料）。
    pub(crate) const fn new() -> Self {
        Self {
            maps: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// 收帧（`SpaceInner::unmap` / `Map::carve` 调用）。
    pub(super) fn take_map(&mut self, map: Map) {
        self.maps.push(map);
    }

    /// 收段（拆除入口在校验通过后调用）。
    pub(super) fn take_span(&mut self, span: Span) {
        self.spans.push(span);
    }

    /// 结清：清退本空间 ASID → 还段 → 帧 drop。顺序即安全性。
    ///
    /// 空料箱直接返回（无易主 = 无清退义务），故装配回滚路径零成本。
    ///
    /// 前置：调用时**不得持 Space 锁**（本方法自行取锁还段；清退在锁外）。
    ///
    /// # Errors
    ///
    /// [`Deaf`] = 某核未在耐心内到齐（致命级，适配层裁定策略）。
    pub(crate) fn reclaim(mut self, space: &Space) -> Result<(), Deaf> {
        let maps = core::mem::take(&mut self.maps);
        let spans = core::mem::take(&mut self.spans);
        if maps.is_empty() && spans.is_empty() {
            return Ok(());
        }
        evict::evict(space.asid())?;
        space.with(|inner| {
            for span in &spans {
                let ok = inner.deallocate(span.seg, span.va.as_usize(), span.size.get());
                debug_assert!(ok, "salvage: segment mismatch on reclaim {:?}", span.va);
            }
        });
        drop(maps);
        Ok(())
    }
}

impl Drop for Salvage {
    fn drop(&mut self) {
        debug_assert!(
            self.maps.is_empty() && self.spans.is_empty(),
            "salvage dropped unreclaimed: {} maps, {} spans",
            self.maps.len(),
            self.spans.len()
        );
    }
}

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

    /// 立即装配（Eager）：登记全物化 Map + 逐页 [`Self::frame`]() 分配帧、
    /// 装叶、注入（帧自产，物理可断）。
    ///
    /// 中途帧耗尽回滚**装配**（清已装叶 + 摘自身 Map）。
    pub(crate) fn claim_map(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
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
        self.install(va, pages, flags, MapMode::Claim(va), || Self::frame())
    }

    /// 装配调用方配好的帧（物理可断，逐帧装叶）+ 登记全物化 Map。
    /// 帧随 Map drop 归还。`frames` 非空；失败回滚清已装叶。
    pub(crate) fn attach_map(
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
    pub(crate) fn borrow_map(
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

    /// 统一拆除 `[va, va+size)`：清相交已物化叶 PTE + 回收变空的中间表；
    /// **全覆盖**的 Map 整张摘除、**部分覆盖**的 Map 按洞分裂——摘下的帧一律
    /// **交料箱**（`salvage`），清退到齐后才归还（远核可能仍持旧条目）。不碰段。
    pub(crate) fn unmap(&mut self, va: VirtAddr, size: usize, salvage: &mut Salvage) {
        if size == 0 {
            return;
        }
        // 1. 先清已物化 PTE（map 还在册，能判哪些页已物化；懒区按帧数走）。
        //    必须**先于**摘 map——摘后 map 消失，clear 找不到覆盖 map，
        //    PTE 残留成悬垂（指向已归还帧，复用即错乱）。
        self.clear(va, size);
        // 2. 摘/裂 map（帧交料箱）
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
            if va.as_usize() <= s && end >= m_end {
                salvage.take_map(m); // 全覆盖：整张摘除交料箱
                continue;
            }
            // 部分覆盖：挖洞分裂（洞内帧由 carve 交料箱）
            let lo_pg = (lo - s) / PAGE_SIZE;
            let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
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

    /// 清 `[va, va+size)` 内的叶 PTE + 回收变空的中间表。
    ///
    /// 一次调用 [`TableNode::unmap`] 完成完整清理（含清叶 + 拆空中间表）；
    /// 未物化页 `walk_mut(false)` 缺失中间表返回 NotMapped = 与无映射一致，
    /// 直接跳过——不需按稀疏帧表预过滤。
    pub(crate) fn clear(&mut self, va: VirtAddr, size: usize) {
        if size == 0 {
            return;
        }
        self.root.unmap(va, size);
    }

    // ── 帧 ──────────────────────────────────────────────────

    /// 领一帧（零化 + Task 类别标注）——全模块唯一帧分配点。
    ///
    /// 类别 = Task：懒/堆/栈/COW 帧属任务生命周期——关机归零。
    pub(crate) fn frame() -> Result<Frame, MapError> {
        let frame: Frame = unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        };
        // 类别 = Task：懒/堆/栈/COW 帧属任务生命周期——关机归零。
        Ok(crate::tag!(Task, frame))
    }

    // ── 物化 / 保护 / 共享 ──────────────────────────────────

    /// 懒页物化：查 Lazy 映射 → 分配零页 → 装叶 + 注入帧。
    ///
    /// `pending` 非 Lazy（Guard / None）或无映射 → 错误。
    /// 循环失败时 [`InstallGuard`] 按 [`MapMode::Materialize`] 自动拆 PTE +
    /// 摘 frames 键；收尾成功须 `commit()` 拆雷。
    pub(crate) fn materialize_map(&mut self, va: VirtAddr, size: usize) -> Result<(), MapError> {
        let pages = size.div_ceil(PAGE_SIZE);
        // 前置：从已存在的 Lazy map 拿 flags + 校验 pending
        let flags = {
            let m = self.resolve_ref(va).ok_or(MapError::NoRegion)?;
            if m.pending != Some(Pending::Lazy) {
                return Err(MapError::NoRegion);
            }
            m.flags | PteFlags::A | PteFlags::D
        };
        self.install(va, pages, flags, MapMode::Materialize, || Self::frame())
    }

    /// 修改已映射区域的保护标志：按物化态分流——全物化（含借用）→ `walk_mut`
    /// 翻叶 PTE；懒区 → 已触页翻叶 + 同步 flags、未触页只同步 flags；
    /// guard → 只同步 flags。
    pub(crate) fn protect(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        let flags = flags | PteFlags::V;
        let pages = size.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let m_va = va + i * PAGE_SIZE;
            let (pending, materialized) = {
                let m = self.resolve_ref(m_va).ok_or(MapError::NoRegion)?;
                let idx = (m_va.as_usize() - m.va.as_usize()) / PAGE_SIZE;
                (m.pending, m.is_materialized(idx))
            };
            if materialized {
                // 改叶 PTE flags——下沉到 TableNode
                self.root.protect(m_va, PAGE_SIZE, flags)?;
            }
            if pending == Some(Pending::Lazy) {
                self.resolve_mut(m_va).expect("map exists").flags = flags;
            }
        }
        Ok(())
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
                // 类别 = Task：COW 共享帧（Shared）属任务生命周期——关机归零。
                let mut arc: Arc<[u8; PAGE_SIZE], &'static dyn alloc::alloc::Allocator> = crate::tag!(
                    Task,
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
                    // 类别 = Task：COW 分裂新帧属任务生命周期——关机归零。
                    let mut nb: Frame = Self::frame()?;
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

    /// 簿记↔页表一致性核对（boot / 压力测试后调用；不一致即 panic）。
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

// SAFETY: 全部可变状态由 `RelLock` 互斥；页表树读写与 `SpaceInner` 共享同一把锁。
unsafe impl Send for Space {}
unsafe impl Sync for Space {}

/// 虚拟地址空间（布局随运行模式）。
///
/// 拥有根页表与**全部自有物理帧**。全部可变状态（`SpaceInner`：root / user /
/// kernel / maps）收进一把 [`RelLock`]。
///
/// # Concurrency
///
/// 全部可变状态收进一把 [`RelLock`]（跨 hart 自旋、同 hart 可重入）。窗口事务
/// 经 [`with`](Self::with) / [`with_flush`](Self::with_flush) 锁恰好一次。**段表
/// 并入本锁**：`Segment` 无锁，所有 `allocate`/`deallocate` 必须在事务内。
///
/// # Drop
///
/// `root`（页表树，递归归还全部表帧）、`maps` 帧随字段自动 drop 归还 frame 池
/// ——所有权驱动。
#[derive(Debug)]
pub struct Space {
    /// 全部可变状态（root / 两段 / maps）——一把可重入锁保护。
    inner: RelLock<SpaceInner>,
    /// 空间种类（内核 / 用户），内嵌 ASID。
    kind: SpaceKind,
}

/// 连续 VA 区间逐段翻译迭代器（`Space::segments` 产出）。
///
/// 每步：经 `Space::translate` 译当前 VA 页 → 产出 `(PA, flags, 页内余量)`。
/// 物理帧可能不连续，故按页切段；未映射页 → 迭代终止。惰性零分配。
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
        let (pa, flags) = self.space.translate(VirtAddr::from_raw(self.va))?;
        let page = self.va & !(PAGE_SIZE - 1);
        let chunk = (page + PAGE_SIZE - self.va).min(self.end - self.va);
        self.va += chunk;
        Some((pa, flags, chunk))
    }
}

/// [`Space`] 构造器。
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

    /// 用户空间构造器（独立 ASID）。
    pub fn user() -> Self {
        Self {
            kind: SpaceKind::User {
                asid: asid::allocate(),
            },
        }
    }

    /// 完成构建：分配根页表帧；用户空间额外种入 trampoline 叶 PTE。
    pub fn build(self) -> Result<Space, MapError> {
        let mut space = Space {
            kind: self.kind,
            inner: RelLock::new_level(Level::Space, SpaceInner::durable()?),
        };
        if matches!(space.kind, SpaceKind::User { .. }) {
            self.seed_user(&mut space)?;
        }
        Ok(space)
    }

    /// 从内核地址空间出用户空间（`build()` 内部调用）。
    ///
    /// 不复制内核半区映射——用户页表只含用户映射 + trampoline 叶 PTE 复制
    /// （帧归内核，借用映射：`pending: None` + 空帧）。
    fn seed_user(&self, space: &mut Space) -> Result<(), MapError> {
        let (tramp_pa, tramp_flags) = {
            let ks_inner = crate::work::unit::team::kernel()
                .expect("kernel team not initialized")
                .space
                .inner
                .lock();
            ks_inner.root.walk_ref(TRAMPOLINE)?
        };
        space.borrow_map(TRAMPOLINE, tramp_pa, PAGE_SIZE, tramp_flags)?;
        Ok(())
    }
}

impl Space {
    // ── 身份 ────────────────────────────────────────────────

    /// 空间种类（内核 / 用户）——任务模式判定的单一事实源。
    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    /// 本空间的 ASID（写入 `satp.ASID` 用；0 = 内核空间）。
    pub fn asid(&self) -> usize {
        self.kind.asid()
    }

    /// 返回根页表页号（写入 `satp` 用）。
    pub fn root(&self) -> usize {
        self.with(|inner| inner.root.ppn())
    }

    // ── 事务入口 ────────────────────────────────────────────

    /// 簿记事务：锁恰好一次，闭包内直接操作 [`SpaceInner`]；锁外不刷 TLB。
    ///
    /// 纪律：闭包内**不得再调用 `Space` 的任何方法**（重入双 guard，UB）。
    /// **段表并入本锁**——`allocate`/`deallocate` 只能在此闭包内发生。
    pub(crate) fn with<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let mut inner = self.inner.lock();
        let r = op(&mut inner);
        drop(inner);
        r
    }

    /// 写页表事务（新增 / 放宽 / 换帧）：同 [`with`](Self::with)，锁外按本空间
    /// ASID 刷 TLB。**无远核义务**——远核最坏持陈旧无效条目或旧窄权限条目，
    /// 会吃一次伪缺页，由 `fault` 的 re-walk 判 resolved + trap 两侧整表刷自愈。
    pub(crate) fn with_flush<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let r = self.with(op);
        // SAFETY: sfence.vma 见 flush_asid
        unsafe {
            flush_asid(self.kind.asid());
        }
        r
    }

    /// 写页表事务（**收紧**：降权 / 只读化）：同 [`with`](Self::with)，锁外
    /// **就地跨核清退**——远核仍持旧宽权限条目 = 写不缺页 = 丢失更新。
    ///
    /// # Errors
    ///
    /// [`Deaf`] = 某核未在耐心内到齐（致命级，适配层裁定策略）。
    pub(crate) fn with_evict<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> Result<R, Deaf> {
        let r = self.with(op);
        evict::evict(self.kind.asid())?;
        Ok(r)
    }

    // ── 适配层（转发 inner 原语 + 刷）───────────────────────

    /// 只登记簿记（懒/守卫/借用占位），不装 PTE，不需刷 TLB。
    pub(crate) fn map(
        &self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Result<(), MapError> {
        self.with(|inner| inner.map(va, size, flags, pending))
    }

    /// 借帧连续映射（DRAM 恒等 / trampoline / dock·ring 视图）。
    pub fn borrow_map(
        &self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.with_flush(|inner| inner.borrow_map(vaddr, paddr, size, flags))
    }

    /// 已备帧装配（loader 逐段 / hart 帧 / health 压测）。
    pub(crate) fn attach_map(
        &self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.with_flush(|inner| inner.attach_map(vaddr, frames, flags))
    }

    /// 统一拆除（munmap 后端 / guard 打洞）：清叶 + 摘/裂 Map + 刷本核 + 结清
    /// （清退到齐后帧才归还）。不还段——段由 [`Self::release`] 收。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        let mut salvage = Salvage::new();
        self.with_flush(|inner| inner.unmap(vaddr, size, &mut salvage));
        salvage.reclaim(self).expect("unmap: evict deaf");
    }

    /// Span 回收门：校验段 + 统一拆除 + 刷本核 + 结清（清退到齐后**才**还段与帧
    /// ——还段即 VA 可复用，远核旧条目会污染新映射）。
    ///
    /// `Span` 由 claim/allocate/mmap 产出（`release` 只收它——分配与回收同一
    /// 类型，杜绝 re-find）。失败域：`MapError::SegmentMismatch` = Span 与段状态
    /// 不一致（调用方 bug——绝大多数调用方用 `.expect()` 保留 panic 语义）。
    pub(crate) fn release(&self, span: Span) -> Result<(), MapError> {
        let mut salvage = Salvage::new();
        self.with_flush(|inner| {
            // 1. 只读校验（失败即 SegmentMismatch，状态未动）
            if !inner.holds(span.seg, span.va.as_usize(), span.size.get()) {
                return Err(MapError::SegmentMismatch);
            }
            // 2. 统一拆除（清叶 / 摘·裂 map，帧交料箱）+ 段一并入箱
            inner.unmap(span.va, span.size.get(), &mut salvage);
            salvage.take_span(span);
            Ok(())
        })?;
        salvage.reclaim(self).expect("release: evict deaf");
        Ok(())
    }

    /// 懒页物化（缺页处理：分配零页装叶注入 + 刷 TLB）。
    pub fn materialize_map(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| inner.materialize_map(vaddr, size))
    }

    /// 修改保护标志（mprotect 后端）：收紧类，就地跨核清退。
    pub fn protect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        self.with_evict(|inner| inner.protect(vaddr, size, flags))
            .expect("protect: evict deaf")
    }

    /// 共享只读化（COW fork 前置：Owned → Shared）：收紧类，就地跨核清退。
    #[allow(dead_code)] // fork 后端预留
    pub fn share(&self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_evict(|inner| inner.share(start, size))
            .expect("share: evict deaf")
    }

    /// 写时分裂私有（COW 写缺页：Shared → 新 Owned；写缺页调用传 `PAGE_SIZE`）。
    /// 非 Shared 页静默跳过——与 [`Self::share`] 跳过非 Owned 对称。
    ///
    /// 抵押（fork 未接通期成立）：归"只刷本核"档的前提是 `share` 仍是 dead code
    /// ——今天没有任何页是 Shared，本方法不可达。fork 接通后同空间多线程会出现
    /// 「他核持旧共享帧的只读陈旧条目 → 读不到本核的写」，届时必须改走
    /// [`Self::with_evict`]。
    #[allow(clippy::wrong_self_convention)] // Space 跨核 Arc 共享，&self 刻意为之
    pub fn own(&self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| inner.own(start, size))
    }

    /// 判别 va 所在页是否 Shared 共享态。
    pub fn is_shared(&self, va: VirtAddr) -> bool {
        let page = va.page_align();
        self.with(|inner| {
            let Some(map) = inner.resolve_ref(page) else {
                return false;
            };
            let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
            matches!(map.frames.get(&idx), Some(FrameState::Shared(_)))
        })
    }

    // ── 查询 ────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位（页表读路径）。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.with(|inner| inner.translate(vaddr))
    }

    /// 查询 `vaddr` 所属映射的物化态（缺页分派用）。
    pub fn pending_state(&self, vaddr: VirtAddr) -> PendingState {
        self.with(|inner| match inner.resolve_ref(vaddr) {
            Some(m) => match m.pending {
                None => PendingState::Materialized,
                Some(Pending::Lazy) => PendingState::Lazy,
                Some(Pending::Guard) => PendingState::Guard,
            },
            None => PendingState::Absent,
        })
    }

    /// 连续 VA 区间 → 逐段物理翻译（[`Segments`] 迭代器）。
    ///
    /// 逐页步进（物理帧可能不连续），产出 `(物理地址, 标志, 段字节数)`；
    /// 某页未映射即停。
    pub fn segments(&self, va: VirtAddr, len: usize) -> Segments<'_> {
        Segments {
            space: self,
            va: va.as_usize(),
            end: va.as_usize() + len,
        }
    }

    /// 页表树节点总数（health 自测用，debug-only——非审计链）。
    #[cfg(debug_assertions)]
    pub fn table_count(&self) -> usize {
        self.with(|inner| inner.root.count())
    }

    /// 簿记↔页表一致性审计（boot / 压力测试后调用；不一致即 panic）。
    #[cfg(feature = "audit")]
    pub(crate) fn audit(&self) {
        self.with(|inner| inner.audit());
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        // 先释放本空间的 ASID（内含清退：ASID 立即可被复用，残留条目会让新
        // 空间同 VA 命中旧映射）。此路径恒走快路径——Arc 归零 ⇒ 无任务持有本
        // 空间 ⇒ 没有任何核驻留该 ASID。
        if let SpaceKind::User { asid } = self.kind {
            asid::deallocate(asid).expect("space drop: evict deaf");
        }
        // `inner` 随字段自动 drop：root（页表树）/maps 帧全部归还 frame 池。
    }
}
