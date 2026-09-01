// core — 主类型 [`Space`] / [`SpaceBuilder`] / [`SpaceInner`] + 映射原语
//
// # 簿记模型（纯映射簿记，无业务语义）
//
// `SpaceInner` 四件：页表树（[`TableNode`]）+ 两段（user 半区 / kernel 帧区，
// 见 [`Segment`]）+ 唯一映射表（[`Map`]）。窗口适配层（`window/`）只组合
// `Space` 的通用原语表达领域策略——`Space` 不知道栈/帧/堆/mmap 是什么。
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
// 无锁，全部 `alloc`/`deallocate` 必须发生在 `with`/`with_flush` 事务内——
// 任何绕开 `Space::with` 的取段/还段都是破坏锁序（死锁/数据竞争）。
//
// 借用约定：guard 是 `Deref`，方法调用的自动引用会借整个 deref 目标——需要
// 同时借 `durable` 的不同字段时，先绑定局部变量（字段级拆借），再调用方法。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE, TRAMPOLINE};
use crate::lock::{Level, RelLock};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::{Frame, FrameState, TableNode};
use crate::memory::manager::{asid, flush_asid, mode};

use super::map::{Map, Pending};
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

/// 地址空间的可变状态 — 由 [`Space::inner`] 这把 [`RelLock`] 保护。
///
/// 纯映射簿记：页表树 + 两段 + 唯一映射表。`user` 段在装载/内核 init 前未
/// 就绪（`None`），经 [`Space::attach_free`] 设置一次；`kernel` 段为布局
/// 常量域构造即定。
pub(crate) struct SpaceInner {
    /// 页表树（翻译基础，全空间一棵）。
    pub(crate) root: TableNode,
    /// 用户半区段 `[free_base, upper)` — 栈/堆/dock 同池（attach_free 前 None）。
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
    pub(crate) fn new() -> Result<Self, MapError> {
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

    /// 设置 user 段边界 `[base, upper)`（恰好一次；任何 user 段分配之前）。
    ///
    /// # Panics
    ///
    /// 重复设置（user 段已在册）或 base 非页对齐 / 越过 upper。
    pub(crate) fn attach_free(&mut self, base: usize) {
        assert!(self.user.is_none(), "Space: user segment double attach");
        let edge = mode::upper().as_usize();
        assert!(
            base.is_multiple_of(PAGE_SIZE) && base <= edge,
            "Space: bad user segment [{base:#x}, {edge:#x})"
        );
        self.user = Some(super::seg::Segment::new(base, edge));
    }

    /// 清 `[va, va+size)` 内**已物化页**的叶 PTE + 回收变空的中间表。
    ///
    /// 懒区只有已触页有 PTE/帧：按覆盖 map 的已登记帧数逐页走（O(触页数)，
    /// 非 O(段大小)）——1 TiB 级懒区不可逐页扫全段。帧已随 map 摘除 drop 归还。
    pub(crate) fn clear_ptes(&mut self, va: VirtAddr, size: usize) {
        if size == 0 {
            return;
        }
        // 按覆盖区间的已物化页数清叶 PTE
        let end = va.as_usize().saturating_add(size);
        for m in &self.maps {
            let s = m.va.as_usize();
            let m_end = s.saturating_add(m.size.get());
            // 与 [va, end) 相交的已物化页
            let lo = va.as_usize().max(s);
            let hi = end.min(m_end);
            if lo >= hi {
                continue;
            }
            for i in (lo - s) / PAGE_SIZE..(hi - s).div_ceil(PAGE_SIZE) {
                if m.is_materialized(i) {
                    self.root.unmap(VirtAddr::from_raw(s + i * PAGE_SIZE));
                }
            }
        }
        // 单次回收变空的中间表
        let geo = mode::geometry(mode::mode());
        let mask = (1usize << geo.va_bits) - 1;
        self.root.reclaim(
            (geo.levels - 1) as usize,
            0,
            va.as_usize() & mask,
            end & mask,
        );
    }

    /// 把调用方配好的物理帧映射到一段连续虚拟地址（常数侧装配，物理可断）。
    ///
    /// 逐帧装 PTE（每帧自身 PA）+ 登记一张多页 Map（`pending: None` = 全物化）。
    /// 帧随 Map drop 归还。`vaddr` 必须按页对齐；`frames` 非空。
    pub(crate) fn map_frames(
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
        for (i, frame) in frames.iter().enumerate() {
            let pa = PhysAddr::from_raw(frame.as_ptr() as usize);
            self.root.map(vaddr + i * PAGE_SIZE, pa, PAGE_SIZE, flags)?;
        }
        self.maps.push(Map::new(
            vaddr,
            size,
            flags,
            None, // Eager（全物化）
            frames.into_iter().map(FrameState::Owned).collect(),
        ));
        Ok(())
    }

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
/// 并入本锁**：`Segment` 无锁，所有 `alloc`/`deallocate` 必须在事务内。
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
        space.map(TRAMPOLINE, tramp_pa, PAGE_SIZE, tramp_flags, Vec::new())?;
        Ok(())
    }
}

impl Space {
    // ── 身份 ──────────────────────────────────────────────────

    /// 空间种类（内核 / 用户）——任务模式判定的单一事实源。
    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    /// 本空间的 ASID（写入 `satp.ASID` 用；0 = 内核空间）。
    pub fn asid(&self) -> usize {
        self.kind.asid()
    }

    // ── 事务入口 ──────────────────────────────────────────────

    /// 簿记事务：锁恰好一次，闭包内直接操作 [`SpaceInner`]；锁外不刷 TLB。
    ///
    /// 纪律：闭包内**不得再调用 `Space` 的任何方法**（重入双 guard，UB）。
    /// **段表并入本锁**——`alloc`/`deallocate` 只能在此闭包内发生。
    pub(crate) fn with<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let mut inner = self.inner.lock();
        let r = op(&mut inner);
        drop(inner);
        r
    }

    /// 写页表事务：同 [`with`](Self::with)，锁外按本空间 ASID 刷 TLB。
    pub(crate) fn with_flush<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let r = self.with(op);
        // SAFETY: sfence.vma 见 flush_asid
        unsafe {
            flush_asid(self.kind.asid());
        }
        r
    }

    // ── 段设置 ────────────────────────────────────────────────

    /// 设置 user 段边界（装载 / 内核 init 时恰好一次；任何 user 段分配之前）。
    pub(crate) fn attach_free(&self, base: usize) {
        self.with(|inner| inner.attach_free(base));
    }

    // ── 映射原语 ──────────────────────────────────────────────

    /// 从段取一块 VA（lowest first-fit）。
    ///
    /// # Errors
    ///
    /// 段未就绪（user 未 attach）→ [`MapError::NoRegion`]；段空隙不足 →
    /// [`MapError::OutOfMemory`]。
    pub(crate) fn alloc(&self, seg: Seg, size: usize) -> Result<VirtAddr, MapError> {
        self.with(|inner| {
            let base = match seg {
                Seg::User => inner
                    .user
                    .as_mut()
                    .ok_or(MapError::NoRegion)?
                    .allocate(size)
                    .map_err(|_| MapError::OutOfMemory)?,
                Seg::Kernel => inner
                    .kernel
                    .allocate(size)
                    .map_err(|_| MapError::OutOfMemory)?,
            };
            Ok(VirtAddr::from_raw(base))
        })
    }

    /// 登记一张 map（VA 已从段取出；只改簿记，不装 PTE）。
    pub(crate) fn register(
        &self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Result<(), MapError> {
        self.with(|inner| {
            if size == 0
                || !va.as_usize().is_multiple_of(PAGE_SIZE)
                || !size.is_multiple_of(PAGE_SIZE)
            {
                return Err(MapError::NotAligned);
            }
            if inner.overlaps(va, size) {
                return Err(MapError::AlreadyMapped);
            }
            inner
                .maps
                .push(Map::new(va, size, flags, pending, Vec::new()));
            Ok(())
        })
    }

    /// 释放一段 VA：归还段 + 摘覆盖 map（帧随 drop 归还）+ 清已物化 PTE + 刷 TLB。
    ///
    /// `Span` 由 claim/allocate/mmap 产出（`release` 只收它——分配与回收同一
    /// 类型，杜绝 re-find）。段归还失败（精确匹配不中）即 **panic**：绝不静默
    /// 泄漏（Span 与分配恒一致，失败 = 调用方错误）。
    pub(crate) fn release(&self, span: Span) {
        self.with_flush(|inner| {
            // 1. 先清已物化 PTE（map 还在册，能判哪些页已物化；懒区按帧数
            //    O(触页数)）。必须**先于摘 map**——摘后 map 消失，clear_ptes
            //    找不到覆盖 map，PTE 残留成悬垂（指向已归还帧，复用即错乱）。
            inner.clear_ptes(span.va, span.size.get());
            // 2. 摘覆盖 map（帧随 drop 归还 frame 池）
            let end = span.va.as_usize().saturating_add(span.size.get());
            inner.maps.retain(|m| {
                !(span.va.as_usize() < m.va.as_usize().saturating_add(m.size.get())
                    && end > m.va.as_usize())
            });
            // 3. 归还段（精确匹配失败 = 调用方错误 → panic，防静默泄漏）
            match span.seg {
                Seg::User => {
                    let user = inner.user.as_mut().expect("user segment attached");
                    assert!(
                        user.deallocate(span.va.as_usize(), span.size.get()),
                        "release: user segment deallocate mismatch for span {:#x}+{}",
                        span.va.as_usize(),
                        span.size.get()
                    );
                }
                Seg::Kernel => {
                    assert!(
                        inner.kernel.deallocate(span.va.as_usize(), span.size.get()),
                        "release: kernel segment deallocate mismatch for span {:#x}+{}",
                        span.va.as_usize(),
                        span.size.get()
                    );
                }
            }
        });
    }

    /// 映射 `size` 字节虚拟地址到物理地址（常数映射，物理连续）。
    ///
    /// PTE 安装 + 登记常数 [`Map`] 一次完成。`frames` 为本映射**拥有**的帧
    /// （借用映射——DRAM 恒等、trampoline 叶——传空，`pending` 自动为 None）。
    ///
    /// **vaddr、paddr、size 必须全部按 [`PAGE_SIZE`] 对齐**，且不得重叠。
    pub fn map(
        &self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
        frames: Vec<Frame>,
    ) -> Result<(), MapError> {
        self.with_flush(|inner| {
            if size == 0
                || vaddr.offset() != 0
                || !paddr.is_aligned()
                || size & (PAGE_SIZE - 1) != 0
            {
                return Err(MapError::NotAligned);
            }
            if inner.overlaps(vaddr, size) {
                return Err(MapError::AlreadyMapped);
            }
            inner.root.map(vaddr, paddr, size, flags)?;
            inner.maps.push(Map::new(
                vaddr,
                size,
                flags,
                None, // 全物化（借用映射空帧）
                frames.into_iter().map(FrameState::Owned).collect(),
            ));
            Ok(())
        })
    }

    /// 取消映射一段虚拟地址并移除其簿记（munmap 后端）。
    ///
    /// 页表侧清叶 PTE + 回收变空的中间表；簿记侧移除被 `[start, start+size)`
    /// **完全覆盖**的映射（帧随 drop 归还）。部分重叠的映射保留记录、仅清 PTE。
    pub fn unmap(&self, vaddr: VirtAddr, size: usize) {
        self.with_flush(|inner| {
            inner.clear_ptes(vaddr, size);
            let end = vaddr.as_usize().saturating_add(size);
            let covered = |m: &Map| {
                vaddr.as_usize() <= m.va.as_usize()
                    && end >= m.va.as_usize().saturating_add(m.size.get())
            };
            inner.maps.retain(|m| !covered(m));
        });
    }

    /// 声明一段预留虚拟映射（懒 Anonymous：首访缺页按 `pending` 分配零页）。
    pub fn declare(
        &self,
        start: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Pending,
    ) -> Result<(), MapError> {
        self.register(start, size, flags, Some(pending))
    }

    /// 修改已映射区域的保护标志（mprotect）。
    ///
    /// 按物化态分流：全物化（含借用）→ `walk_mut` 翻叶 PTE；懒区 → 已触页翻
    /// 叶 + 同步 flags、未触页只同步 flags；guard → 只同步 flags。
    pub fn mprotect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        self.with_flush(|inner| {
            let flags = flags | PteFlags::V;
            let pages = size.div_ceil(PAGE_SIZE);
            for i in 0..pages {
                let va = vaddr + i * PAGE_SIZE;
                let idx = (va.as_usize() - vaddr.as_usize()) / PAGE_SIZE;
                let (pending, materialized) = {
                    let m = inner.resolve_ref(va).ok_or(MapError::NoRegion)?;
                    (m.pending, m.is_materialized(idx))
                };
                if materialized {
                    // 已物化页必有叶：不分配中间表（false），缺失即错误
                    let leaf = inner.root.walk_mut(va, false, mode::levels())?;
                    leaf.set_flags(flags);
                }
                if pending == Some(Pending::Lazy) {
                    inner.resolve_mut(va).expect("map exists").flags = flags;
                }
            }
            Ok(())
        })
    }

    // ── COW（copy-on-write 共享帧）────────────────────────────

    /// 把 `[start, start+size)` 内可写页提升为共享只读：Owned → Borrowed(Arc)。
    #[allow(dead_code)] // fork 后端预留
    pub fn borrow(&self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| {
            for i in 0..size.div_ceil(PAGE_SIZE) {
                let va = start + i * PAGE_SIZE;
                let (bytes_src, flags) = {
                    let map = inner.resolve_ref(va).ok_or(MapError::NotMapped)?;
                    let idx = (va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                    match &map.frames[idx] {
                        FrameState::Borrowed(_) => continue,
                        FrameState::Owned(b) => {
                            let bytes: &[u8] = &**b;
                            (bytes, map.flags)
                        }
                    }
                };
                // 类别 = Task：COW 共享帧（Borrowed）属任务生命周期——关机归零。
                let mut arc: Arc<[u8; PAGE_SIZE], &'static dyn alloc::alloc::Allocator> =
                    crate::tag!(Task, Arc::new_in([0u8; PAGE_SIZE], crate::memory::allocator::frame::allocator()));
                Arc::get_mut(&mut arc)
                    .expect("fresh arc")
                    .copy_from_slice(bytes_src);
                let arc_pa = PhysAddr::from_raw(Arc::as_ptr(&arc) as usize);
                {
                    let map = inner.resolve_mut(va).ok_or(MapError::NotMapped)?;
                    let idx = (va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                    let old = core::mem::replace(&mut map.frames[idx], FrameState::Borrowed(arc));
                    drop(old);
                    let leaf = inner.root.walk_mut(va, false, mode::levels())?;
                    let ppn = (arc_pa.as_usize() >> 12) as u64;
                    leaf.set(
                        ppn,
                        (flags & !PteFlags::W) | PteFlags::A | PteFlags::D | PteFlags::V,
                    );
                }
            }
            Ok(())
        })
    }

    /// 写缺页分裂：保证该页私有可写。Borrowed → 分新 Owned 拷字节。
    #[allow(clippy::wrong_self_convention)] // Space 跨核 Arc 共享，&self 刻意为之
    pub fn to_mut(&self, va: VirtAddr) -> Result<(), MapError> {
        let page = va.page_align();
        self.with_flush(|inner| {
            let map = inner.resolve_mut(page).ok_or(MapError::NotMapped)?;
            let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
            let flags = map.flags;
            if let FrameState::Owned(_) = &map.frames[idx] {
                let leaf = inner.root.walk_mut(page, false, mode::levels())?;
                leaf.set_flags(leaf.flags() | PteFlags::W | PteFlags::V);
                return Ok(());
            }
            let new_box: Frame = {
                let FrameState::Borrowed(arc) = &map.frames[idx] else {
                    unreachable!("checked above")
                };
                // 类别 = Task：COW 分裂新帧属任务生命周期——关机归零。
                let mut nb: Frame = crate::tag!(Task, unsafe {
                    Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                });
                nb.copy_from_slice(&arc[..]);
                nb
            };
            let pa = PhysAddr::from_raw(new_box.as_ptr() as usize);
            let old = core::mem::replace(&mut map.frames[idx], FrameState::Owned(new_box));
            drop(old);
            let leaf = inner.root.walk_mut(page, false, mode::levels())?;
            let ppn = (pa.as_usize() >> 12) as u64;
            leaf.set(ppn, flags | PteFlags::W | PteFlags::V);
            Ok(())
        })
    }

    /// 判别 va 所在页是否 Borrowed 共享态。
    pub fn is_borrowed(&self, va: VirtAddr) -> bool {
        let page = va.page_align();
        self.with(|inner| {
            let Some(map) = inner.resolve_ref(page) else {
                return false;
            };
            let idx = (page.as_usize() - map.va.as_usize()) / PAGE_SIZE;
            matches!(map.frames.get(idx), Some(FrameState::Borrowed(_)))
        })
    }

    /// 放下 `[start, start+size)` 内共享引用（保留为 borrow 的反向语义锚点）。
    #[allow(dead_code)] // 语义由 Map teardown 承担；borrow 成对锚点
    pub fn unborrow(&self, start: VirtAddr, size: usize) {
        let _ = (&self, start, size);
    }

    // ── 缺页处理 ──────────────────────────────────────────────

    /// 缺页处理：查 Lazy 映射 → 分配零页 → 映射 + 注入帧。
    ///
    /// `pending: Some(Lazy)` 的未物化页物化零页；`Guard` / 无映射 → 错误。
    pub fn page_fault(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| {
            let pages = size.div_ceil(PAGE_SIZE);
            for i in 0..pages {
                let va = vaddr + i * PAGE_SIZE;
                let (pending, flags) = {
                    let m = inner.resolve_ref(va).ok_or(MapError::NoRegion)?;
                    (m.pending, m.flags | PteFlags::A | PteFlags::D)
                };
                if pending != Some(Pending::Lazy) {
                    // Guard → 预留触碰；None → 不该缺页（内核 bug）
                    return Err(MapError::NoRegion);
                }
                // 类别 = Task：懒页物化帧属任务生命周期——关机归零。
                let page: Frame = crate::tag!(Task, unsafe {
                    Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                });
                let pa = PhysAddr::from_raw(page.as_ptr() as usize);
                inner.root.map(va, pa, PAGE_SIZE, flags)?;
                let map = inner.resolve_mut(va).expect("map exists (checked above)");
                map.inject(page);
            }
            Ok(())
        })
    }

    /// 查询 `vaddr` 所属映射的物化态（缺页分派用）。
    ///
    /// `Some(Some(Lazy))` → 物化零页；`Some(Some(Guard))` → 预留触碰；
    /// `Some(None)` → 已物化（不该缺页）；`None` → 无映射。
    pub fn resolve_pending(&self, vaddr: VirtAddr) -> Option<Option<Pending>> {
        self.with(|inner| inner.resolve_ref(vaddr).map(|m| m.pending))
    }

    // ── 查询 ──────────────────────────────────────────────────

    /// 将虚拟地址翻译为物理地址和标志位（页表读路径）。
    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.with(|inner| inner.translate(vaddr))
    }

    /// 无锁页表读翻译（诊断/打印路径专用）：不经 `inner` 锁，直接读页表树。
    ///
    /// 仅当本空间装配完成后不再写（内核空间）或在故障现场（其他核已停）可安全
    /// 使用；正常路径必须走 [`translate`](Self::translate)（取锁）。
    ///
    /// # Safety
    /// 调用方须保证此刻无并发写 `inner`（页表树不在变动）。
    pub(crate) unsafe fn translate_unlocked(
        &self,
        vaddr: VirtAddr,
    ) -> Option<(PhysAddr, PteFlags)> {
        // SAFETY: 调用方保证无并发写；只读 walk 页表树。
        let inner = unsafe { &*self.inner.read_unlocked() };
        inner.translate(vaddr)
    }

    /// 返回根页表页号（写入 `satp` 用）。
    pub fn root(&self) -> usize {
        self.with(|inner| inner.root.ppn())
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
        self.with(|inner| {
            for m in &inner.maps {
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
            }
        });
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        // 先释放本空间的 ASID。
        if let SpaceKind::User { asid } = self.kind {
            asid::deallocate(asid);
        }
        // `inner` 随字段自动 drop：root（页表树）/maps 帧全部归还 frame 池。
    }
}
