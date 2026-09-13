// Map — VA→PA 簿记的原子单元。
//
// 语义：本映射覆盖 VA 区间 `[va, va + size)`；`frames` 是**页序 → 帧**的
// 稀疏表（键 i ↔ VA `va + i·PAGE_SIZE`；未物化页不在表——懒映射按触页登记、
// guard 页永不入帧）。`pending` 表达**未物化页的行为**（与帧所有权正交）：
//   None       — 无未物化页（全物化）。拥有映射满帧；借用映射（DRAM 恒等、
//                 trampoline、dock 视图）空帧——leaf 在册、物理帧归外部。
//   Some(Lazy) — 未物化页缺页时分配零页（懒映射：`ShareWindow::mmap` 的共享区；
//                 栈体相反，是**立即物化**，见 `window/stack.rs`）。
//   Some(Guard)— 未物化且禁止物化：触碰即「预留映射访问」（栈守护页）。
//
// 帧随 Map drop 归还 frame 池——所有权即回收，无遍历页表树、无手写 deallocate
// （只读借用页不在 `frames` 里：所有权归那个活得更久的所有者）。
//
// 部分拆除（`carve`）按洞分裂：摘洞内帧、本图就地收缩（洞在头时重绕成右段）。
// 洞是**结构上的**（键区间缺省），不是额外 holes 字段；稀疏表让
// `is_materialized` / `clear` 只随已物化键走（O(触页数)，非 O(段大小)）。
//
// # 两处形态是为「增长可失败」选的，不是为省内存
//
// - `Frames` 用有序 `Vec` 而不是 `BTreeMap`：`BTreeMap` 没有 `try_reserve`
//   （本地 rust-src 零命中），于是每次首插都是一次**不可失败**的叶节点分配
//   （实测 368 B = 16 + 88 + 11×24，落在缺页物化路径上）。`Vec` 能先把容量备足
//   再插（`Frames::reserve`），失败当场答 `OutOfMemory`，机器照旧活着；
//   尺寸不变（`(usize, Frame)` = 32 B/条，叶节点今天约 33 B/条）。
// - `next` 让摘下的图能**整体搬进料箱**而不必往容器里推（`Salvage`）：拆除路径
//   （`release`）不能失败——它在调用方是 `.expect()`，给释放路径引入新失败域
//   等于把 halt 换个位置。故那条路必须零分配，摘下的图靠这条链挂着等清退。

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::Frame;

/// 未物化页的行为（Map 级）— 与帧所有权正交。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    /// 缺页时物化零页（懒分配，帧按触页登记）。
    Lazy,
    /// 禁止物化：触碰即「预留映射访问」（栈守护页）。
    Guard,
}

/// 缺页分派：`va` 所在页的映射/物化态（`pending_state` 返回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingState {
    /// 无映射。
    Absent,
    /// 已物化 / 借用（pending None）——不该缺页。
    Materialized,
    /// 懒：缺页物化零页。
    Lazy,
    /// 守卫：触碰即「预留映射访问」。
    Guard,
}

/// 自有帧表 — 已物化页 → 帧，**按页序**（稀疏：未物化页不在表）。
///
/// 页序不是装饰：`runs` 靠它把连续页折叠成段（调用次数随**触页段数**走、不随区间
/// 大小走），`remove` / `contains` 靠它二分。唯一的分配点是 [`Self::reserve`]。
#[derive(Debug)]
pub(crate) struct Frames {
    v: Vec<(usize, Frame)>,
}

impl Frames {
    pub(super) const fn new() -> Self {
        Self { v: Vec::new() }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.v.is_empty()
    }

    /// 备足 `add` 条余量 —— **本结构唯一会分配的一步**，失败即 `OutOfMemory`。
    ///
    /// 契约：调用方在**动任何状态之前**把它做完（`install` 在装第一页之前一次备足
    /// 本次的 `pages`；`unmap` 的"先备后动"同理）。此后 `insert` / `move_*` 不再分配。
    pub(super) fn reserve(&mut self, add: usize) -> Result<(), MapError> {
        self.v.try_reserve(add).map_err(|_| MapError::OutOfMemory)
    }

    /// 插一帧（前置：容量已备、键不重复）。**不分配**。
    pub(super) fn insert(&mut self, page: usize, frame: Frame) {
        debug_assert!(
            self.v.len() < self.v.capacity(),
            "frames: insert without reserve (page {page})"
        );
        let at = self.v.partition_point(|(k, _)| *k < page);
        assert!(
            self.v.get(at).is_none_or(|(k, _)| *k != page),
            "frames: double insert @page {page}"
        );
        self.v.insert(at, (page, frame));
    }

    /// 摘一帧（无则 `None`）。**不分配**。
    pub(super) fn remove(&mut self, page: usize) -> Option<Frame> {
        let at = self.v.partition_point(|(k, _)| *k < page);
        matches!(self.v.get(at), Some((k, _)) if *k == page).then(|| self.v.remove(at).1)
    }

    /// 该页是否在册（唯一读者是 [`Self::is_materialized`] 的 Lazy 支，故门同它）。
    #[cfg(any(debug_assertions, feature = "framework"))]
    pub(super) fn contains(&self, page: usize) -> bool {
        let at = self.v.partition_point(|(k, _)| *k < page);
        matches!(self.v.get(at), Some((k, _)) if *k == page)
    }

    /// 页序迭代（键 + 帧）。**不分配**。
    ///
    /// 门跟着读者走：唯一读者是 `SpaceInner::audit`（`debug_assertions` /
    /// `framework` 两档），release 档下它连类型都不该被编出来——否则就是零警告
    /// 纪律下的一个死方法。
    #[cfg(any(debug_assertions, feature = "framework"))]
    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, &Frame)> {
        self.v.iter().map(|(k, f)| (*k, f))
    }

    /// `[lo, hi)` 内的条目（页序）。**不分配**。
    pub(super) fn range(&self, lo: usize, hi: usize) -> impl Iterator<Item = (usize, &Frame)> {
        let from = self.v.partition_point(|(k, _)| *k < lo);
        let to = self.v.partition_point(|(k, _)| *k < hi);
        self.v[from..to].iter().map(|(k, f)| (*k, f))
    }

    /// `[lo, hi)` 内的条数（拆除路径"先备后动"的计数用）。**不分配**。
    pub(super) fn count_range(&self, lo: usize, hi: usize) -> usize {
        self.range(lo, hi).count()
    }

    /// 把 `[lo, hi)` 搬进 `to`，页号减 `shift`（洞的键重排）。
    ///
    /// 前置：`to` 容量已备 ⇒ **不分配**。
    pub(super) fn move_range(&mut self, lo: usize, hi: usize, shift: usize, to: &mut Frames) {
        let from = self.v.partition_point(|(k, _)| *k < lo);
        let mut left = self.v.partition_point(|(k, _)| *k < hi) - from;
        debug_assert!(
            to.v.capacity() - to.v.len() >= left,
            "frames: move_range without reserve"
        );
        while left > 0 {
            let (k, f) = self.v.remove(from); // 序号 from 恒指本段首：逐条摘
            to.v.push((k - shift, f));
            left -= 1;
        }
    }

    /// 把**页号 `from` 起（含）**的全部条目搬进 `to`，页号减 `key_sub`（右段）。
    ///
    /// `from` 是**页号**不是下标：稀疏表里"第几个条目"与"第几页"不是一回事，而调用方
    /// （`carve`）手上只有页号。前置同 [`Self::move_range`] ⇒ **不分配**。
    pub(super) fn move_tail(&mut self, from: usize, key_sub: usize, to: &mut Frames) {
        let at = self.v.partition_point(|(k, _)| *k < from);
        debug_assert!(
            to.v.capacity() - to.v.len() >= self.v.len() - at,
            "frames: move_tail without reserve"
        );
        while self.v.len() > at {
            let (k, f) = self.v.remove(at);
            to.v.push((k - key_sub, f));
        }
    }

    /// 洞在头：本图重绕成右段，全部键原位减 `sub`。**不分配**。
    pub(super) fn shift_keys(&mut self, sub: usize) {
        for (k, _) in self.v.iter_mut() {
            *k -= sub;
        }
    }
}

/// 虚拟→物理映射 — 簿记的原子单元。
///
/// 语义见文件头。`frames` 稀疏（只含已物化页）；`pending` 描述未物化页行为。
#[derive(Debug)]
pub(crate) struct Map {
    /// 拆除链（**只在料箱手里**；在册的图恒 `None`）。
    ///
    /// 链头归 `Salvage`：链上的图已从 `SpaceInner::maps` 摘除，其帧要等跨核清退到齐
    /// 才归池，故必须整张挂着——这就是"拆除路径零分配"的落点。
    pub(super) next: Option<Box<Map>>,
    pub(super) va: VirtAddr,
    pub(super) size: NonZeroUsize,
    pub(super) flags: PteFlags,
    /// 未物化页行为（None = 全物化：拥有满帧 / 借用空帧）。
    pub(super) pending: Option<Pending>,
    /// 已物化页帧（页序键；未物化页不在表）。
    pub(super) frames: Frames,
}

impl Drop for Map {
    /// **迭代**摘链：`next` 是 `Option<Box<Map>>`，默认的递归 drop 会把长链压进
    /// 调用栈（一次 munmap 拆下几百张图不是异常）。逐节 take 出来即自然展开。
    fn drop(&mut self) {
        let mut cur = self.next.take();
        while let Some(mut node) = cur {
            cur = node.next.take();
        }
    }
}

impl Map {
    /// 构造（size 必须非零——调用方保证，见各入口的校验）。帧表初始为空。
    pub(super) fn new(va: VirtAddr, size: usize, flags: PteFlags, pending: Option<Pending>) -> Self {
        Self {
            next: None,
            va,
            size: NonZeroUsize::new(size).expect("map size must be non-zero"),
            flags,
            pending,
            frames: Frames::new(),
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    pub(super) fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 第 `idx` 页是否已物化（已在页表）——**点查询**。
    ///
    /// - `pending: None` → 全物化（拥有映射满帧 / 借用映射 leaf 在册）→ true。
    /// - `Some(Lazy)` → 已触页（`frames` 含键）→ true；未触 → false。
    /// - `Some(Guard)` → 永不物化 → false。
    ///
    /// 运行路径（拆除 / 改权）不用点查询逐页问，用 [`Self::runs`] 的段枚举。
    #[cfg(any(debug_assertions, feature = "framework"))]
    pub(super) fn is_materialized(&self, idx: usize) -> bool {
        match self.pending {
            None => true,
            Some(Pending::Lazy) => self.frames.contains(idx),
            Some(Pending::Guard) => false,
        }
    }

    /// 本映射是否为**借入**（物理帧归外部所有）。
    ///
    /// 判据就是文件头那句"借用映射空帧"：**全物化的空帧表**。三态完备、无歧义：
    /// - 满帧（拥有映射）⇒ 自有；
    /// - `Lazy` / `Guard` ⇒ 自有：懒区缺页时自己分配零页，guard 永不物化
    ///   （借用映射恒 `pending: None`——leaf 立刻在册，没有"未物化页"可言）；
    /// - 空帧 + `pending: None` ⇒ **借入**，且必然全物化（惰性态不在链中）。
    ///
    /// 用途：改权限时的**所有权闸**——借入页只能收紧、不能加宽。
    pub(super) fn is_borrowed(&self) -> bool {
        self.pending.is_none() && self.frames.is_empty()
    }

    /// 为本次装配**一次备足** `pages` 条帧表余量（唯一会分配的一步，失败即
    /// `OutOfMemory`）。契约：调用方在装第一页之前调它，此后 `Frames::insert` 不分配。
    pub(super) fn reserve_frames(&mut self, pages: usize) -> Result<(), MapError> {
        self.frames.reserve(pages)
    }

    /// 造一张与本图**同属性**的图，覆盖自 `first_pg` 页起的 `pages` 页——拆除时
    /// 分裂出来的洞图 / 右段图（`unmap` 的"先备后动"用它备料）。
    ///
    /// 分配两笔：图对象本身 + 其帧表余量 `frames` 条；都失败即 `OutOfMemory`。
    /// 调用方必须在**动任何状态之前**调它，这样失败时簿记一字未改。
    pub(super) fn part(
        &self,
        first_pg: usize,
        pages: usize,
        frames: usize,
    ) -> Result<Box<Map>, MapError> {
        let mut map = Map::new(
            self.va + first_pg * PAGE_SIZE,
            pages * PAGE_SIZE,
            self.flags,
            self.pending,
        );
        map.reserve_frames(frames)?;
        Box::try_new(map).map_err(|_| MapError::OutOfMemory)
    }

    /// 把 `[lo_pg, hi_pg)` 内**已物化**的页折叠成连续段，逐段交 `apply`——
    /// [`Self::is_materialized`] 的**区间版**，同一判据的唯一另一种形态。
    ///
    /// 借用/全物化 = 整段一次；懒区 = `frames` 的键（页序，天然可折叠）；
    /// guard = 空集。折叠即复杂度：调用次数随**触页段数**走、不随区间大小走
    /// （1 TiB 懒区触 4 页 = 1 段），拆除/改权因此与段大小解耦。
    pub(super) fn runs(&self, lo_pg: usize, hi_pg: usize, mut apply: impl FnMut(VirtAddr, usize)) {
        match self.pending {
            None => apply(self.va + lo_pg * PAGE_SIZE, (hi_pg - lo_pg) * PAGE_SIZE),
            Some(Pending::Lazy) => {
                let mut run: Option<(usize, usize)> = None;
                for (pg, _) in self.frames.range(lo_pg, hi_pg) {
                    run = match run {
                        Some((start, len)) if start + len == pg => Some((start, len + 1)),
                        Some((start, len)) => {
                            apply(self.va + start * PAGE_SIZE, len * PAGE_SIZE);
                            Some((pg, 1))
                        }
                        None => Some((pg, 1)),
                    };
                }
                if let Some((start, len)) = run {
                    apply(self.va + start * PAGE_SIZE, len * PAGE_SIZE);
                }
            }
            Some(Pending::Guard) => {}
        }
    }

    /// 把洞 `[lo_pg, hi_pg)`（页序，半开）挖掉——统一拆除中**部分覆盖**的 Map。
    ///
    /// 本图**就地收缩**，三种位置各有落法（与旧版按值分裂等价，但现在不造临时表）：
    /// - 洞在头（`lo_pg == 0`）：本图重绕成右段（键原位减 `hi_pg`），`right` 必须为 `None`；
    /// - 洞在尾（`hi_pg == pages`）：本图留左段，`right` 必须为 `None`；
    /// - 洞在中间：本图留左段，右段帧搬进 `right`（键减 `hi_pg`）。
    ///
    /// `hole`：洞内有帧时给出**预置好的图**（几何已定、帧表容量已备），洞内帧搬进去
    /// ——它随即整张交料箱（清退到齐后才归还，不得在此 drop：远核可能仍持旧条目）；
    /// 洞内无帧（借入页 / 未触页）时传 `None`。
    ///
    /// 前置：`lo_pg < hi_pg ≤ pages`、**非全覆盖**（全覆盖由调用方直接整张摘除），
    /// 且两个目标帧表的容量已由调用方备足（`unmap` 的"先备后动"）⇒ **本函数不分配**。
    pub(super) fn carve(
        &mut self,
        lo_pg: usize,
        hi_pg: usize,
        hole: Option<&mut Map>,
        right: Option<&mut Map>,
    ) {
        let pages = self.size.get() / PAGE_SIZE;
        debug_assert!(lo_pg < hi_pg && hi_pg <= pages);
        if let Some(hole) = hole {
            self.frames.move_range(lo_pg, hi_pg, lo_pg, &mut hole.frames);
        }
        if lo_pg == 0 {
            // 洞在头：本图重绕成右段（洞口那截已搬走，余下键原位减 hi_pg）
            debug_assert!(right.is_none(), "carve: 洞在头时由本图重绕，无独立右段");
            self.frames.shift_keys(hi_pg);
            self.va += hi_pg * PAGE_SIZE;
            self.size = NonZeroUsize::new((pages - hi_pg) * PAGE_SIZE).expect("non-zero");
            return;
        }
        self.size = NonZeroUsize::new(lo_pg * PAGE_SIZE).expect("non-zero");
        if let Some(right) = right {
            debug_assert!(hi_pg < pages, "carve: 洞在尾时无独立右段");
            self.frames.move_tail(lo_pg, hi_pg, &mut right.frames);
        }
    }
}
