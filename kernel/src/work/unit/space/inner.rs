// inner — [`SpaceInner`] + 全部映射原语（Space 内的这一半：无锁无刷）
//
// # 层职责（同一把锁的两半）
//
// - [`SpaceInner`] = 内：数据 + 全部操作体，**无锁无刷**（只出现在事务闭包内）。
// - [`Space`] = 外（`outer.rs`）：`RelLock` 门（[`with`]/[`with_flush`]）+ 每操作 ≤3 行转发；
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
    /// 用户半区段 `[free_base, upper)` — 栈/堆/共享页同池（dynamic 前 None）。
    pub(crate) user: Option<super::segment::Segment>,
    /// 内核 trap 帧常量区段 `[TEAM_FRAME_BASE, +SIZE)`，S-only。
    pub(crate) kernel: super::segment::Segment,
    /// 唯一 VA→PA 簿记（单表遍历，无常数/动态之分）。
    ///
    /// 元素是 `Box<Map>` 而不是 `Map`：拆除路径要能把摘下的图**整体搬进料箱**
    /// （[`Salvage`] 的链，零分配）——值住在 `Vec` 的缓冲里就搬不走。
    #[allow(clippy::vec_box)]
    pub(crate) maps: Vec<Box<Map>>,
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
        // （`map` / `claim` / `attach` / `borrow` / 各窗口的 claim 都在这条路上），
        // 而那次 `Vec::push` 的扩容若走 std 默认路径就是 `handle_alloc_error` →
        // 整机 halt。**预留现在紧贴每个 push 点**——`self.maps.try_reserve(1)` 本文件
        // 只有 `register` 一处；`unmap` 备的是 `try_reserve(rights)`，`splits` 那张表
        // 另算。此前它只在本函数里备一格，而 `StackWindow::claim` 一个入口要推两张
        // （guard + 栈体），于是"预留 1 格、推 2 张"，扩容就落在 churn 现场那次
        // 917504 B 上（`= 8192×112` 是过期标定，`Map` 实测 64 B；照实记）。一次预留
        // 配一次增长，且两者挨在一起。
        let base = match seg {
            SegmentKind::Normal => self
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
            SegmentKind::Normal => match self.user.as_mut() {
                Some(u) => u,
                None => return false,
            },
            SegmentKind::Kernel => &mut self.kernel,
        };
        seg.deallocate(addr, size)
    }

    // ── 装配族 ──────────────────────────────────────────────

    /// 注册一张图：**先备表容量、再造对象、最后 push**——三步里唯一会分配的两步
    /// 都在此处失败（都答 `OutOfMemory`），失败时表与段一个字都没动。
    ///
    /// 这是"一次预留配一次增长"的落点：预留**紧贴** `push`，调用方不再需要在更远处
    /// 预判自己会推几张。
    fn register(&mut self, map: Map) -> Result<(), MapError> {
        self.maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let map = Box::try_new(map).map_err(|_| MapError::OutOfMemory)?;
        self.maps.push(map);
        Ok(())
    }

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
        self.register(Map::new(va, size, flags, pending))
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
        self.register(Map::new(va, size, flags, None))?;
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
        self.register(Map::new(vaddr, size, flags, None))?;
        let mut iter = frames.into_iter();
        self.install(vaddr, pages, flags, MapMode::Claim(vaddr), move || {
            Ok(iter.next().expect("attach: frame iter exhausted"))
        })
    }

    /// 借帧连续映射：物理地址已知、一次装连续段，不持帧（帧归外部：机器/
    /// 内核/`PoleMeta`）。DRAM 恒等、trampoline、Pole 共享页视图走这。
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
        self.register(Map::new(vaddr, size, flags, None))
    }

    // ── 拆除 ────────────────────────────────────────────────

    /// 统一拆除 `[va, va+size)`：逐 map 清其相交且**真有 PTE** 的页（中间表随之
    /// 回收）→ **全覆盖**的 Map 整张摘除交料箱、**部分覆盖**的按洞分裂（本图就地
    /// 收缩，洞在头时重绕成右段）——摘下的图一律**交料箱**（`salvage`），清退到齐后
    /// 才归还（远核可能仍持旧条目）。不碰段（段由 [`Space::release`] 收）。
    ///
    /// # 先备后动（本函数的硬约束）
    ///
    /// 这是 `MemoryCall::Deallocate` 的必经之路——**释放不得依赖内存**。旧版
    /// `mem::take` + 新建 `survivors` 每次调用都重建整张映射表：表涨到 512 条时
    /// `grow_one` 那一步就是一次 **114688 B（28 页）** 的请求（现场数据，按旧标定的
    /// `Map` = 112 B 记），而它落在内存已经吃紧的释放路径上——压垮内核的那次分配，
    /// 是「释放」自己递上去的（`trace/churn6/console.log` 的实测现场）。今天的
    /// `Map` 实测 64 B（next 8 + va 8 + size 8 + flags 8 + pending 1 + 补齐 7 +
    /// `Frames(Vec)` 24）；那个 112 B 是过期标定（照实记）。
    ///
    /// 现在两趟走。**第一趟只读**：数出这次要造几张图（洞图 / 右段图）、各要搬
    /// 多少帧，并**当场把容量备足**——任何一步失败都在**状态一字未动**时答
    /// `OutOfMemory`（照 `cull.rs:51` 的教训：让 `try_reserve` 在拆到一半时才失败，
    /// 就把"摘一半"变成可达状态）。**第二趟只搬**：`Vec::remove` 等长压实 + 帧表
    /// `move_*` + 链上挂图，一次分配都没有。
    ///
    /// 于是 `Space::release` 那条**不能失败**的路（调用方是 `.expect()`）恒走
    /// "无需造图"的分支（它按 Span 精确归还，块内的图恒被整覆盖）⇒ 零分配 ⇒ 永不失败。
    ///
    /// # Errors
    ///
    /// 备料失败 → [`MapError::OutOfMemory`]（状态未动，可安全重试）。
    pub(crate) fn unmap(
        &mut self,
        va: VirtAddr,
        size: usize,
        salvage: &mut Salvage,
    ) -> Result<(), MapError> {
        if size == 0 {
            return Ok(());
        }
        let lo = va.as_usize();
        let end = lo.saturating_add(size);

        // ── 第一趟：只读 + 备料 ──────────────────────────────
        //
        // `splits[i]` 与第二趟里第 i 张**部分覆盖**的图逐一同位：两趟对"全覆盖 /
        // 部分覆盖 / 不相交"用同一套判据，且第二趟只做等长压实、不改相互次序。
        let mut splits: Vec<Split> = Vec::new();
        let mut rights = 0usize;
        for m in self.maps.iter() {
            let Some((lo_pg, hi_pg)) = intersect(m, lo, end) else {
                continue;
            };
            let s = m.va.as_usize();
            if lo <= s && end >= s.saturating_add(m.size.get()) {
                continue; // 全覆盖：整张进料箱，不造新图
            }
            let pages = m.size.get() / PAGE_SIZE;
            // 洞内有帧才需要洞图：借入页 / 未触页的洞没有帧要挂着等清退。
            let hole = {
                let n = m.frames.count_range(lo_pg, hi_pg);
                (n > 0)
                    .then(|| m.part(lo_pg, hi_pg - lo_pg, n))
                    .transpose()?
            };
            // 右段：洞**在中间**时才独立成一张图（在头由本图重绕、在尾无右段）。
            let right = if lo_pg != 0 && hi_pg < pages {
                Some(m.part(hi_pg, pages - hi_pg, m.frames.count_range(hi_pg, pages))?)
            } else {
                None
            };
            rights += usize::from(right.is_some());
            splits.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
            splits.push(Split { hole, right });
        }
        // 第二趟会把右段图推回表：先把追加格备足（此后 `push` 不再增长）。
        self.maps
            .try_reserve(rights)
            .map_err(|_| MapError::OutOfMemory)?;

        // ── 第二趟：只搬 ────────────────────────────────────
        let SpaceInner { root, maps, .. } = self; // 字段级拆借：清叶要用 root
        let mut i = 0usize;
        let mut n = 0usize;
        while i < maps.len() {
            let m_va = maps[i].va.as_usize();
            let m_size = maps[i].size.get();
            let l = lo.max(m_va);
            let h = end.min(m_va.saturating_add(m_size));
            if l >= h {
                i += 1; // 不相交（含第二趟刚推回表的左段/右段：它们已不含本区间）
                continue;
            }
            let lo_pg = (l - m_va) / PAGE_SIZE;
            let hi_pg = (h - m_va).div_ceil(PAGE_SIZE);
            // 清叶与摘图同一趟：拿着图才能问它哪些页有 PTE，顺序不可能写反。
            // `remove` 是等长压实（后继整体左移），故此处**不递增**下标。
            let mut m = maps.remove(i);
            m.runs(lo_pg, hi_pg, |rva, rsize| root.unmap(rva, rsize));
            if lo <= m_va && end >= m_va.saturating_add(m_size) {
                salvage.take_map(m); // 全覆盖：整张挂上料箱链（零分配）
                continue;
            }
            let split = &mut splits[n];
            n += 1;
            m.carve(
                lo_pg,
                hi_pg,
                split.hole.as_deref_mut(),
                split.right.as_deref_mut(),
            );
            if let Some(hole) = split.hole.take() {
                salvage.take_map(hole);
            }
            if let Some(right) = split.right.take() {
                maps.push(right);
            }
            maps.push(m);
        }
        debug_assert_eq!(n, splits.len(), "unmap: 两趟的图数不一致");
        Ok(())
    }

    /// 只读校验：`(addr, size)` 是否为该段的一个已分配块（拆除路径的失败域
    /// 前移，见 [`super::segment::Segment::holds`]）。
    pub(crate) fn holds(&self, seg: SegmentKind, addr: usize, size: usize) -> bool {
        match seg {
            SegmentKind::Normal => self.user.as_ref().is_some_and(|u| u.holds(addr, size)),
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
            .filter_map(|m| span(m))
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

    /// 查询 `vaddr` 所属的映射（单表线性查，无常数/动态之分），返回借用。
    pub(super) fn resolve_ref(&self, vaddr: VirtAddr) -> Option<&Map> {
        self.maps
            .iter()
            .rev()
            .find(|m| m.contains(vaddr))
            .map(Box::as_ref)
    }

    /// 查询 `vaddr` 所属映射的可变引用（缺页注入帧用）。
    pub(super) fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        self.maps
            .iter_mut()
            .rev()
            .find(|m| m.contains(vaddr))
            .map(Box::as_mut)
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
    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        for m in &self.maps {
            for (i, f) in m.frames.iter() {
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

/// `unmap` 第一趟为一张**部分覆盖**的图备下的两张图（不需要时 `None`）。
///
/// `hole` 收洞内的帧、随后整张进料箱；`right` 是洞右侧独立成的那张，回表。
/// 备料与搬移分两趟，是为了让"要分配"与"改状态"不交叠：分配失败时状态一字未动。
struct Split {
    hole: Option<Box<Map>>,
    right: Option<Box<Map>>,
}

/// `[lo, end)` 与本图相交的那一段（页序，半开）；不相交 → `None`。
///
/// 两趟（备料 / 搬移）共用这一套算式——写在一处，就不会出现"数的"和"搬的"
/// 不是同一批。
fn intersect(m: &Map, lo: usize, end: usize) -> Option<(usize, usize)> {
    let s = m.va.as_usize();
    let l = lo.max(s);
    let h = end.min(s.saturating_add(m.size.get()));
    (l < h).then(|| ((l - s) / PAGE_SIZE, (h - s).div_ceil(PAGE_SIZE)))
}

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
/// `installed` = 已装页数；`commit()` 归零 → drop 循环 0 次 = no-op。
/// 成功路径必须 `commit()`，否则 drop 会拆掉刚装的页。
///
/// 为什么是一个计数而不是集合：`mark` 只在 [`SpaceInner::install`] 的 `for i in 0..pages`
/// 循环里、按 `0..pages` **单调**调用（`mark` 紧跟在"叶已装 + 帧已注入"之后，中间
/// 没有别的可失败步骤），故已装页恒为**前缀** `[0, installed)`。旧版这里是
/// `BTreeSet<usize>`——每次装配一张页表就多一次不可失败的节点分配，而它要表达的
/// 事实只是"前 k 页装好了"。
struct InstallGuard<'a> {
    inner: &'a mut SpaceInner,
    va: VirtAddr,
    installed: usize,
    book: MapMode,
}

impl<'a> InstallGuard<'a> {
    fn new(inner: &'a mut SpaceInner, va: VirtAddr, book: MapMode) -> Self {
        Self {
            inner,
            va,
            installed: 0,
            book,
        }
    }
    fn mark(&mut self) {
        self.installed += 1;
    }
    /// 拆雷：drop 时 installed=0 不调 unmap；book 不动——Claim 仍按原策略摘整张 map。
    /// 消费 self。
    fn commit(mut self) {
        self.installed = 0;
    }
}

/// 页槽位里那帧的物理地址（恒等映射下指针值即 PA）。
///
/// 独立函数而不是方法：**唯一使用者是 [`SpaceInner::audit`]**，而它整段是
/// `#[cfg(debug_assertions)]`——做成结构上的方法会让
/// 那个字段在没有用例的档里没有任何读点（dead_code 警告），违反"零警告、零
/// allow"（用户裁决）。
#[cfg(debug_assertions)]
fn page_pa(f: &Frame) -> PhysAddr {
    PhysAddr::from_raw(f.as_ptr() as usize)
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        // commit() 已归零 → 成功路径，整个 drop 不做任何动作
        if self.installed == 0 {
            return;
        }
        // 1. 按精确页号清已 set 叶 PTE（单页 unmap，各自拆空中间表）
        for j in 0..self.installed {
            self.inner.root.unmap(self.va + j * PAGE_SIZE, PAGE_SIZE);
        }
        // 2. 按策略动 maps（精确页号——装到哪页就撤哪页）
        match self.book {
            MapMode::Materialize => {
                for j in 0..self.installed {
                    if let Some(m) = self.inner.resolve_mut(self.va + j * PAGE_SIZE) {
                        m.frames.remove(j);
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
    ///
    /// # 帧表容量在装第一页之前一次备足
    ///
    /// `Frames` 的插入要求容量已备（`Vec` 的扩容是不可失败路径）；备料是**唯一**
    /// 会分配的一步，放在循环之前 ⇒ 失败时一页未装、一字未改。
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
        self.resolve_mut(va)
            .expect("map exists")
            .reserve_frames(pages)?;
        let mut guard = InstallGuard::new(self, va, book);
        let result: Result<(), MapError> = (|| {
            for i in 0..pages {
                let m_va = va + i * PAGE_SIZE;
                let page = next_frame()?;
                let pa = PhysAddr::from_raw(page.as_ptr() as usize);
                guard.inner.root.map(m_va, pa, PAGE_SIZE, flags)?;
                let map = guard.inner.resolve_mut(m_va).expect("map exists");
                let idx = (m_va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                map.frames.insert(idx, page);
                guard.mark();
            }
            Ok(())
        })();
        if result.is_ok() {
            guard.commit();
        }
        result
    }
}
