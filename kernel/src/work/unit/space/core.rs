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
use alloc::vec::Vec;

use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::mode;
use crate::memory::manager::table::{Frame, TableNode};

use super::SegmentKind;
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
    pub(crate) user: Option<super::segment::Segment>,
    /// 内核 trap 帧常量区段 `[TEAM_FRAME_BASE, +SIZE)`，S-only。
    pub(crate) kernel: super::segment::Segment,
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
            kernel: super::segment::Segment::new(
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
        self.user = Some(super::segment::Segment::new(base, edge));
    }

    // ── 段轴 ────────────────────────────────────────────────

    /// 从段取一块 VA（lowest first-fit）。
    ///
    /// # Errors
    ///
    /// 段未就绪（user 未 dynamic）→ [`MapError::NoRegion`]；段空隙不足 →
    /// [`MapError::OutOfMemory`]。
    ///
    /// # 取段之后装配失败，善后**只有还段**（那条不变量的唯一出处）
    ///
    /// 本函数是"先占段、后装配"族（`map` / `borrow` / `claim` / `attach`）的第一步。
    /// 装配族失败时，**maps 簿记已由它自己回滚干净**：
    ///
    /// - `map` / `borrow` 在 `push` 之前就拒（`NotAligned` / `AlreadyMapped` / 叶写
    ///   失败）——没登记过，无事可撤；
    /// - `claim` / `attach` 走 `install`，失败时 [`InstallGuard`] 按 `MapMode::Claim`
    ///   清已装叶并摘整张 map（见本文件 `Drop` 分支）。
    ///
    /// 于是调用方在 `Err` 之后**只欠段一笔**，且这笔可以**当场**还（`deallocate`），
    /// 不必绕 [`Salvage`] 的清退：此刻没有任何 PTE 落过，也就没有任何远核能持旧条目
    /// ——"还段即 VA 可复用"这条危险的时序（见 [`Space::release`]）在此不成立。
    ///
    /// 所以每个现场都长这样，一字不改：
    ///
    /// ```ignore
    /// let va = inner.allocate(seg, size)?;
    /// if let Err(e) = inner.<装配>(va, ...) {
    ///     inner.deallocate(seg, va.as_usize(), size);   // 唯一善后 = 还段
    ///     return Err(e);
    /// }
    /// ```
    ///
    /// 这条规则此前没有出处，于是被五个现场各自推理了一遍，其中两处推错：
    /// `StackWindow` 的 body 失败分支展开了整段 `unmap` + `Salvage` + `reclaim`
    /// （那次 `unmap` 的料箱恒空、`reclaim` 恒早返回——纯多余），而
    /// `PoleMeta::open_into` 一个字都没写（段永久泄漏）。写在这里，让后来者不必
    /// 再推第三遍。
    pub(crate) fn allocate(&mut self, seg: SegmentKind, size: usize) -> Result<VirtAddr, MapError> {
        // **映射表的增长可失败**：取段之后必有一张新 `Map` 入 `self.maps`
        // （`map` / `attach` / `borrow` / 各窗口的 claim 都在这条路上），而那次
        // `Vec::push` 的扩容是 std 默认路径——内存吃紧即 `handle_alloc_error`
        // → 整机 halt。在这里先把容量备足，失败就当场答 `OutOfMemory`
        // （`Spawn` / 堆分配把它翻成 `-4` / `-1`），机器照旧活着。
        //
        // 放在取段**之前**：失败时段未被占用，调用方的回滚路径一个字都不用改。
        self.maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let base = match seg {
            SegmentKind::NonKernel => self
                .user
                .as_mut()
                .ok_or(MapError::NoRegion)?
                .allocate(size)
                .map_err(|_| MapError::OutOfMemory)?,
            SegmentKind::Kernel => self
                .kernel
                .allocate(size)
                .map_err(|_| MapError::OutOfMemory)?,
        };
        Ok(VirtAddr::from_raw(base))
    }

    /// 还段：精确匹配释放 `(addr, size)`。未分配 / 长度不匹配 → `false`。
    pub(crate) fn deallocate(&mut self, seg: SegmentKind, addr: usize, size: usize) -> bool {
        let seg = match seg {
            SegmentKind::NonKernel => match self.user.as_mut() {
                Some(u) => u,
                None => return false,
            },
            SegmentKind::Kernel => &mut self.kernel,
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
    ///
    /// # 拆除路径不分配（本函数的硬约束）
    ///
    /// 这是 `MemoryCall::Deallocate` 的必经之路——**释放不得依赖内存**。旧版
    /// `mem::take` + 新建 `survivors` 每次调用都重建整张映射表：`Map` 恰 112 B，
    /// 表涨到 512 条时 `grow_one` 那一步就是一次 **114688 B（28 页）** 的请求，
    /// 而它落在内存已经吃紧的释放路径上——压垮内核的那次分配，是「释放」自己
    /// 递上去的（`trace/churn6/console.log` 的实测现场）。
    ///
    /// 故改为**原地压实**：`retain_mut` 把存活映射就地左移、末尾截断，缓冲区
    /// 自始至终是 `self.maps` 自己那一块；闭包内 `Vec::push` 只用于**拆分**新
    /// 产出的右段（一条映射至多一个，常态零分配）。
    pub(crate) fn unmap(&mut self, va: VirtAddr, size: usize, salvage: &mut Salvage) {
        if size == 0 {
            return;
        }
        let end = va.as_usize().saturating_add(size);
        // 被拆出的右段（`carve` 只在「洞在中间」时产出）：先存本地，遍历结束再
        // 追加——`retain_mut` 期间不得再借 `self.maps`。
        let mut rights: Vec<Map> = Vec::new();
        let root = &mut self.root;
        self.maps.retain_mut(|m| {
            let s = m.va.as_usize();
            let m_end = s.saturating_add(m.size.get());
            let lo = va.as_usize().max(s);
            let hi = end.min(m_end);
            if lo >= hi {
                return true; // 不相交
            }
            let lo_pg = (lo - s) / PAGE_SIZE;
            let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
            // 清叶与摘 map 同一趟：拿着 map 才能问它哪些页有 PTE，顺序不可能写反。
            m.runs(lo_pg, hi_pg, |rva, rsize| root.unmap(rva, rsize));
            if va.as_usize() <= s && end >= m_end {
                // 全覆盖：整张摘除交料箱。`Map` 无 `Default`，用一个占位 Map 换出
                // 真身（占位随即被 retain 截掉）——搬移而非重建。
                let taken = core::mem::replace(
                    m,
                    Map::new(m.va, PAGE_SIZE, m.flags, m.pending, BTreeMap::new()),
                );
                salvage.take_map(taken);
                return false;
            }
            // 部分覆盖：挖洞分裂（洞内帧由 carve 交料箱）。
            match m.carve(lo_pg, hi_pg, salvage) {
                Some(right) => {
                    rights.push(right); // 右段：洞在中间
                    true // 左段（carve 已收缩 m）
                }
                // `None` 有两种来源，必须分开——判据与 `carve` 内部分支逐一对应：
                //   洞在头（`lo_pg == 0`）⇒ **本 map 已被重绕成右段**，无右段产出，
                //                          故它仍是存活映射，保留；
                //   洞在尾（`hi_pg == 页数`）⇒ 右段不存在，本 map（左段）保留。
                None => lo_pg != 0,
            }
        });
        self.maps.append(&mut rights);
    }

    /// 只读校验：`(addr, size)` 是否为该段的一个已分配块（拆除路径的失败域
    /// 前移，见 [`super::seg::Segment::holds`]）。
    pub(crate) fn holds(&self, seg: SegmentKind, addr: usize, size: usize) -> bool {
        match seg {
            SegmentKind::NonKernel => self.user.as_ref().is_some_and(|u| u.holds(addr, size)),
            SegmentKind::Kernel => self.kernel.holds(addr, size),
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
    /// # 所有权闸（借入页只能收紧）
    ///
    /// **借入**映射（`frames` 空、`pending` 无 —— 见 [`Map::is_borrowed`]）的物理帧
    /// 归外部所有，那份映射是别人所有权的**只读借用**（Pole 视图等）。对它，新 flags
    /// 必须是当前 PTE flags 的**子集**；否则拒 [`MapError::WidenDenied`]。
    ///
    /// 为什么必须在这里挡：叶 PTE 是权限的**权威**（`map.flags` 只是指针，且只对
    /// 惰性态同步），而本函数是全树唯一的 flags 写点。放行加宽等于开一个按 VA 就
    /// 能扩大他人资源权限的入口——`narrow` 的 `cap ⊆ 页表` 契约正是靠"加宽无路可走"
    /// 成立的（`Mprotect` 走的就是本函数，它没有、也不该有 pie 句柄）。
    ///
    /// 自有的页（满帧 / 懒区 / guard）不受此限：那是调用方自己的内存，重设权限是
    /// 它的正当用途（如把私有页重新标成可写）。
    ///
    /// # Errors
    ///
    /// - `NoRegion` — 区间内有页不落在任何 map（此时尚未落任何改动）
    /// - `WidenDenied` — 区间内有借入页要被加宽（此时尚未落任何改动）
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
        // 2. 所有权闸先行（同样整体失败、状态未动）：借入页不许被加宽。
        //    用页表当前 flags 判"加宽"（叶 PTE 是权威，不读可能陈旧的 map.flags）。
        {
            let root = &self.root;
            for m in self.maps.iter() {
                if !m.is_borrowed() {
                    continue;
                }
                let Some((s, lo, hi)) = span(m) else { continue };
                let lo_pg = (lo - s) / PAGE_SIZE;
                let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
                let mut denied = false;
                m.runs(lo_pg, hi_pg, |rva, rsize| {
                    for i in 0..(rsize / PAGE_SIZE) {
                        let page = rva + i * PAGE_SIZE;
                        if let Ok((_, cur)) = root.walk_ref(page)
                            && flags.bits() & !cur.bits() != 0
                        {
                            denied = true;
                        }
                    }
                });
                if denied {
                    return Err(MapError::WidenDenied);
                }
            }
        }
        // 3. 落改（root 与 maps 是不同字段，可同时可变借出）
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
                let expect = page_pa(f);
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

/// 页槽位里那帧的物理地址（恒等映射下指针值即 PA）。
///
/// 独立函数而不是方法：**唯一使用者是 [`SpaceInner::audit`]**，而它整段是
/// `#[cfg(feature = "audit")]`——做成结构上的方法会让那个字段在默认档没有任何
/// 读点（dead_code 警告），违反"两档零警告、零 allow"（用户裁决）。
#[cfg(feature = "audit")]
fn page_pa(f: &Frame) -> PhysAddr {
    PhysAddr::from_raw(f.as_ptr() as usize)
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
