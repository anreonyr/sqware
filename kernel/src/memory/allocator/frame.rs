use crate::memory::PAGE_SIZE;
use core::ptr::NonNull;
use erra::ResultExt;

use alloc::{
    alloc::{AllocError, Allocator},
    boxed::Box,
    vec::Vec,
};

use super::fence::checker;
use crate::{
    lock::{Level, OnceLock, SpinLock},
    memory::allocator::{InitError, InitResult, Link, bump},
};

/// 页元数据：free 区每页一条（仅**块首帧**有表项——伙伴块内其余页无独立条目）。
struct Meta {
    free: bool,
    power: u8,
}

impl Meta {
    fn new(free: bool, power: u8) -> Self {
        Self { free, power }
    }
}

pub(crate) struct FrameAllocator {
    inner: SpinLock<FrameInner>,
}

impl FrameAllocator {
    /// 构建 frame 分配器：分配元数据 Vec（经 bump），确定基址，构建 frame freelist。
    ///
    /// 在 `init` 的 OnceLock 顶层单例里运行时装配——故 `inner` 是**无 Option** 的
    /// `SpinLock<FrameInner>`，不存在"未初始化 None"死路（同 block 每节点）。
    fn init() -> Result<Self, InitError> {
        let mut inner = FrameInner::new();
        inner.init()?;
        Ok(Self {
            inner: SpinLock::new_level(Level::Frame, inner),
        })
    }

    /// 帧池窗口 `[base, edge)` 与其中的保留区（洞）列表 —— **扫描类诊断**要按这个
    /// 范围走：内核只映射了自己用的那部分 DRAM，越过窗口或踩进未映射的保留区就是
    /// 一次 page fault（本轮真踩过：扫到 `0x87f90000` 当场 panic）。
    pub(crate) fn window(
        &self,
    ) -> (
        usize,
        usize,
        [Option<(usize, usize)>; crate::machine::MAX_RESERVED],
    ) {
        let g = self.inner.lock();
        let holes = FrameInner::holes(g.base, g.edge, g.pagemeta.len());
        (g.base, g.edge, holes)
    }

    /// 帧分配器 free 区总页数（statistics::init 拍 total / available 用）。
    pub(crate) fn total_pages(&self) -> usize {
        let g = self.inner.lock();
        (g.edge - g.base) / PAGE_SIZE
    }

    /// 该物理页当前是否在帧分配器手里（held）。
    ///
    /// **唯一真相**：`pagemeta`。banker 的每页位图删除后，持久帧登记表与账本落页
    /// 检查都问这里——同一件事不再有两份账。
    /// 消费者只有 audit 档（关机不变量 + boot 三源核对），故同 gate。
    #[cfg(feature = "audit")]
    pub(crate) fn is_held(&self, pa: usize) -> bool {
        let g = self.inner.lock();
        g.held(pa)
    }



    /// 那个"伙伴说空闲但不在链上 ⇒ 放弃合并"分支被走了多少次（探针）。
    ///
    /// 该分支 `break` 后，那个伙伴**既没被合并、也没被重新入链**，而它的
    /// `free=true` 标记留着——正是"`pagemeta` 说空闲、链上却找不到"的来源。
    /// 这个数在 churn 下单调增长即为该机制成立的直接证据。
    pub(crate) fn nomerge_count() -> usize {
        NOMERGE.load(::core::sync::atomic::Ordering::Relaxed)
    }


    /// 释放"在手块中间帧"的累计次数（见 `checker::INTERIOR_FREES`）—— 恒应为 0。
    pub(crate) fn interior_frees() -> usize {
        crate::memory::allocator::fence::checker::INTERIOR_FREES
            .load(::core::sync::atomic::Ordering::Relaxed)
    }

    /// 写点落在别人跨度内的累计次数（块首互不可能包含 ⇒ 恒应为 0）。
    pub(crate) fn covered_writes() -> usize {
        COVERED.load(::core::sync::atomic::Ordering::Relaxed)
    }



    /// **诊断入口**：见 `FrameInner::interior_of_held`。
    pub(crate) fn interior_of_held(&self, pa: usize) -> Option<(usize, u8)> {
        self.inner.lock().interior_of_held(pa)
    }





    /// 独立重算一次水位：走链 + 按**块首步进**扫 `pagemeta`。
    ///
    /// 步进才是 `pagemeta` 的正确读法：表项只存在于**块首**，一个块占 `2^power`
    /// 帧（见 `FrameInner::pagemeta` 的「仅块首帧有表项」）。逐表项求和是**错的**
    /// ——它把逻辑上已被并入大块、但表项尚未清掉的帧重复计入。
    fn tally(g: &FrameInner) -> (usize, usize, usize) {
        let walk = g
            .freelist
            .iter()
            .enumerate()
            .map(|(o, head)| chain_len(*head, g.pagemeta.len() + 1) << o)
            .sum();
        let mut held = 0usize;
        let mut idle = 0usize;
        let mut cursor = 0usize;
        while cursor < g.pagemeta.len() {
            if let Some(m) = g.pagemeta[cursor].as_ref() {
                let frames = 1usize << m.power;
                if m.free {
                    idle += frames;
                } else {
                    held += frames;
                }
                cursor += frames;
            } else {
                cursor += 1;
            }
        }
        (walk, held, idle)
    }

    /// 逐 `Kind` 的**当前在册帧数**（`Trap` / `Stack` / `Table` / `Heap` …）。
    ///
    /// 读数来自 `statistics` 的按类计数（`record_frame_take` / `record_frame_give`
    /// 在帧分配器里**无条件**调用，不受 audit 档影响），故这是可用的**分类水位**。
    ///
    /// **为什么这才是判漏该用的表**：先前我盯的是池总量（`walk` / `held`），那是
    /// 一锅粥——任何一类在漏都表现为"池少了"。逐类看才能直接指到漏的是哪一类，
    /// 而不是逼着人从总量反推机制（我为此编了三个错误机制，浪费了好几轮）。
    ///
    /// 只列非零项；名字走 [`crate::memory::allocator::fence::Kind::name`]，不另维
    /// 一张字符串表（两张表必然漂移）。
    pub(crate) fn kind_counts() -> alloc::string::String {
        let v = crate::memory::allocator::statistics::view_frame();
        let mut out = alloc::string::String::new();
        for (i, n) in v.kinds.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            let kind = crate::memory::allocator::fence::Kind::ALL[i];
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(kind.name());
            out.push('=');
            let _ = core::fmt::Write::write_fmt(&mut out, format_args!("{n}"));
        }
        out
    }

    /// **链结构自审**：逐桶走一遍，核对双向链表的每一环。
    ///
    /// # 为什么必须单独有这样一条判据
    ///
    /// 先前 `prev` 的维护是**静默失效**的：`n.read().prev = None` / `head.read().prev =
    /// Some(addr)` 都是对 `NonNull::read()` 返回的**临时副本**赋值 —— 编译通过、不 panic、
    /// 不报警，唯一的后果要等到某次 `remove_link` 依据 `prev == None` 走错分支、
    /// 把真正的桶头覆盖成陈旧 `next` 时才显现，而那时距肇事处已过了上千次操作。
    /// 这种缺陷**只能**在当场逐环核对时抓住（`[stale-rm]` 是被动取证，本函数是主动判据）。
    ///
    /// # 核对项
    ///
    /// * 桶头 `prev == None`，且 `freelist[o]` 与走链起点一致；
    /// * 每个节点 `n`：`n` 的表项存在、`free == true`、`power == o`；
    /// * `n.next` 的 `prev` 回指 `n`（尾节点 `next == None`）；
    /// * `n.prev` 的 `next` 正指 `n`；
    /// * 不成环、不越界（预算 = 表长 + 1）。
    ///
    /// 返回 `(节点数, 环次数, 不一致数, 首个不一致样本 (帧索引, 桶号, 码))`；
    /// 码：`1` 桶头 `prev` 非 `None`、`2` 表项不符、`3` `next` 不回指、
    /// `4` `prev` 不正指、`5` 越界/成环。
    pub(crate) fn chain_audit(&self) -> (usize, usize, usize, (usize, usize, u8)) {
        let g = self.inner.lock();
        let cap = g.pagemeta.len() + 1;
        let mut nodes = 0usize;
        let mut cycles = 0usize;
        let mut bad = 0usize;
        let mut first = (0usize, 0usize, 0u8);
        let mut note = |code: u8, idx: usize, o: usize, bad: &mut usize, first: &mut (usize, usize, u8)| {
            if *bad == 0 {
                *first = (idx, o, code);
            }
            *bad += 1;
        };
        for (o, head) in g.freelist.iter().enumerate() {
            let mut cur = *head;
            let mut prev_addr: Option<usize> = None;
            let mut budget = cap;
            while let Some(node) = cur {
                if budget == 0 {
                    cycles += 1;
                    note(5, 0, o, &mut bad, &mut first);
                    break;
                }
                budget -= 1;
                nodes += 1;
                let pa = node.as_ptr() as usize;
                if pa < g.base || pa >= g.edge {
                    note(5, 0, o, &mut bad, &mut first);
                    break;
                }
                let idx = (pa - g.base) / PAGE_SIZE;
                // SAFETY: 链节点恒为空闲块，头 16 字节是 `push_link` 写的 `Link`。
                let link = unsafe { &*(pa as *const Link) };
                let pv = link.prev.map(|x| x.as_ptr() as usize);
                let nx = link.next.map(|x| x.as_ptr() as usize);
                if pv != prev_addr {
                    note(4, idx, o, &mut bad, &mut first);
                }
                match g.pagemeta.get(idx).and_then(|m| m.as_ref()) {
                    Some(m) if m.free && m.power as usize == o => {}
                    _ => note(2, idx, o, &mut bad, &mut first),
                }
                if let Some(n) = nx {
                    let nidx = if n >= g.base && n < g.edge {
                        (n - g.base) / PAGE_SIZE
                    } else {
                        usize::MAX
                    };
                    let back = if nidx == usize::MAX {
                        None
                    } else {
                        // SAFETY: 同上，节点地址在窗口内。
                        unsafe { &*(n as *const Link) }.prev.map(|x| x.as_ptr() as usize)
                    };
                    if back != Some(pa) {
                        note(3, nidx, o, &mut bad, &mut first);
                    }
                    prev_addr = Some(pa);
                    cur = NonNull::new(n as *mut Link);
                } else {
                    prev_addr = Some(pa);
                    cur = None;
                }
            }
        }
        (nodes, cycles, bad, first)
    }
}

static NOMERGE: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

/// `remove_link` 里"自称桶头、桶头却不是我"的次数（陈旧 `Link` 导致桶头被覆盖）。
static STALE_RM: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

/// ── 已删：freelist 累计收支探针（`freelist_ledger`）──
///
/// `FR_PUSH/FR_PULL/BK_PUSH/BK_PULL` 四个原子喂的是"封箱 − 开箱 = 当前该在链上的
/// 帧数"那条**第二本守恒账**。探针时代连同 `conserve` 一起撤掉了 —— 现在
/// `walk == idle`（走链 = 表说空闲）由**框架档的 `stress` 用例**间接压住，不再有
/// 逐帧对质的常驻读数。
///
/// 前两版探针都在回收路径上按 `pagemeta.len()` 开 `Vec`，两次都把整机拖进退化态。
/// 这一版**完全不碰堆**。判据：`net_frames` 按**入链时声明的 `power`** 累加，
/// 与走链累加（`walk`，按**桶号**算）对质：
///   · 两者不等 ⇒ 块被放进了**不是它自己 order 的桶**（按桶算自然对不上）；
///   · 两者相等而都远小于 `meta` ⇒ 块确实没进链。
/// `pagemeta` 里各 order 的**空闲块首**条数（探针，固定 17 槽、零堆分配）。
///
/// 判据：它必须等于 `freelist[order]` 的**实际链长**。两者不等 ⇒ 有块被标为空闲
/// 却没进链（幽灵块）—— 而且本计数**只由 `pagemeta` 写点驱动**，不经过任何
/// 遍历，故它自己不会"看不见"东西。
/// **写点判据**：写入一个**已被别条表项跨度覆盖**的索引的次数。
///
/// 块首之间不可能互相包含（每条表项声明"从本索引起、占 `2^power` 帧"），所以这个数
/// **必须恒为 0**。实测非零且有稳定样本（当年由 `stress::chain()` 用例当哨兵看着；
/// 该用例已随帧池探针一并撤）。
static COVERED: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);




/// 数一条 order 链的块数（只读遍历，`None` 结尾）。
///
/// # 必须有步数上限
///
/// 旧版没有上限：链若**成环**（同一块入链两次 / `next` 指回前驱），这个循环
/// 永不返回——而它在**持 `inner` 锁**的情况下跑，于是整机在分配器里静默卡死。
/// 实测：诊断路径加上走链读数后，一次 `churn` 从 3.4 s 变成**跑不完**，与
/// "循环不返回"的症状一致。
///
/// 上限取 `pagemeta.len() + 1`（合法块数不可能超过总帧数），超限即按"链坏"处理：
/// 停止遍历 —— "链成环"从"卡死"变成"走链数偏小"。**成环不再单独计数**：
/// （撤掉的 `conserve` 曾用同一份预算，两个计数会漂移 —— 记一笔当时的口径问题。）
fn chain_len(mut n: Option<NonNull<Link>>, cap: usize) -> usize {
    let mut blocks = 0usize;
    let mut budget = cap;
    while let Some(node) = n {
        if budget == 0 {
            break;
        }
        budget -= 1;
        blocks += 1;
        // SAFETY: freelist 节点恒为空闲块，头 16 字节是 Link（prev/next）。
        n = unsafe { node.read() }.next;
    }
    blocks
}


/// 由请求字节数计算 frame order（块 = 2^power × PAGE_SIZE，须覆盖 size）。
///
/// size 先向上取整到页，再取整到 **2 的幂页数**——frame 块必须是 2 的幂倍页。
/// 例：8976 B（3 页）→ 4 页 → power 2（16 KiB ≥ 8976）。
fn block_power(size: usize) -> usize {
    size.max(PAGE_SIZE)
        .next_multiple_of(PAGE_SIZE)
        .next_power_of_two()
        .ilog2() as usize
        - PAGE_SIZE.ilog2() as usize
}

unsafe impl Allocator for FrameAllocator {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        {
            let this = &self;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let mut guard = this.inner.lock();
            let frame = &mut *guard;
            let index = unsafe { frame.split_block(power) }.ok_or(AllocError)?;
            let addr = frame.frame_addr(index) as *mut u8;
            checker::check_dram_addr(addr as usize, "frame alloc (split result)");
            #[cfg(feature = "audit")]
            {
                let a = addr as usize;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame alloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            // 取出即已由 pagemeta 证明"这帧原先 free"（pop_link 的 check_frame_free
            // 在 audit 档也跑）——banker 删除后不再有第二份每页位图。
            super::statistics::record_frame_take(super::fence::Kind::Plain);
            checker::log_frame_alloc(addr as usize, index, power);
            Ok(NonNull::slice_from_raw_parts(
                NonNull::new(addr).ok_or(AllocError)?,
                size,
            ))
        }
    }

    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        unsafe {
            let mut guard = self.inner.lock();
            let frame = &mut *guard;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let addr = ptr.addr().get();
            // 护栏：释放地址必须在 free 区（双释放/错地址释放即刻暴露）
            checker::check_dram_addr(addr, "frame dealloc (ptr)");
            #[cfg(feature = "audit")]
            {
                let a = addr;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame dealloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            let index = frame.frame_index(addr);
            // 护栏：释放的帧必须**仍在手**（pagemeta 唯一真相，O(order) 遍历
            // ——与 check_frame_free 同档：debug + audit；产品档整句不编译，
            // 故那次遍历不付）。
            #[cfg(any(debug_assertions, feature = "audit"))]
            checker::check_frame_held(frame.held(addr), index, addr, power);
            // 判据：被命中的那个"在手大块"（base / bpower / 在手帧数）。命中前后各取一次，
            // **差即答案**：若该块的在手帧数不变（且仍被算作在手），说明释放它的中间帧
            // 并没有把它还回池子 ⇒ 真缺陷；若整块回到池里，则这是"逐帧拆块"的正常用法，
            // 我这轮的判据方向就是错的。
            #[cfg(any(debug_assertions, feature = "audit"))]
            let hit = frame.interior_of_held(addr);
            #[cfg(any(debug_assertions, feature = "audit"))]
            checker::check_frame_head(hit, index, addr, power);

            // 护栏事件：帧存入金库。
            super::fence::on_frame_free(addr);
            frame.merge_block(index, power);
            // 种类：untag 在 fence::on_frame_free 内完成,kind 由 untag 路径同步 record;
            // 非 audit 时 fence::on_frame_free 内部直接 record_frame_give(Plain)。
            // 此处无需再调 record_frame_give,避免重复。

            checker::log_frame_dealloc(addr, index, power);
        }
    }
}

/// 帧索引 `index` 落在哪条保留区内 → 该保留区的**洞尾**（帧索引，开区间）。
///
/// 表已升序、区内不重叠（`Machine::reserved` 的两段物理区互不相交）。
fn hole_containing(holes: &[Option<(usize, usize)>], index: usize) -> Option<usize> {
    holes
        .iter()
        .flatten()
        .find(|&&(start, end)| index >= start && index < end)
        .map(|&(_, end)| end)
}

struct FrameInner {
    freelist: Vec<Option<NonNull<Link>>>,
    pagemeta: Vec<Option<Meta>>,
    base: usize,
    edge: usize,
}

impl FrameInner {
    const fn new() -> Self {
        Self {
            freelist: Vec::new(),
            pagemeta: Vec::new(),
            base: 0,
            edge: 0,
        }
    }

    /// 初始化：分配元数据 Vec（经 bump），确定基址，构建 frame freelist。
    ///
    /// # Errors
    ///
    /// - 空闲区不足一页 → [`InitError::NoFreeFrames`]（`max_frame == 0` 时
    ///   `ilog2` 会 panic，必须提前报错）。
    /// - 元数据 Vec 分配失败 → [`InitError::OutOfMemory`]。
    fn init(&mut self) -> Result<(), InitError> {
        // 第一步：分配 freelist/pagemeta Vecs（基于当前 frontier 暂估尺寸）
        self.edge = bump::boundary();
        let prov_base = bump::frontier().next_multiple_of(PAGE_SIZE);
        let max_frame = self.edge.saturating_sub(prov_base) / PAGE_SIZE;
        if max_frame == 0 {
            return Err(InitError::NoFreeFrames);
        }
        let max_power = max_frame.ilog2() as usize + 1;
        self.freelist
            .try_reserve(max_power)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freelist.resize_with(max_power, || None);
        self.pagemeta
            .try_reserve(max_frame)
            .map_err(|_| InitError::OutOfMemory)?;
        self.pagemeta.resize_with(max_frame, || None);

        // 第二步：此时所有 bump 分配已完成，确定实际基址并收缩 Vec。
        // base ≥ prov_base（frontier 单调前进）⇒ 本步尺寸 ≤ 第一步，
        // resize 不会触发新分配，无需 try_reserve。
        self.base = bump::frontier().next_multiple_of(PAGE_SIZE);
        let max_frame = self.edge.saturating_sub(self.base) / PAGE_SIZE;
        if max_frame == 0 {
            return Err(InitError::NoFreeFrames);
        }
        let max_power = max_frame.ilog2() as usize + 1;
        self.freelist.resize_with(max_power, || None);
        self.pagemeta.resize_with(max_frame, || None);

        let mut index = 0usize;
        let mut remaining = max_frame;
        // 持久保留区（initrd / 设备树……——`Machine::reserved` 一张账）的帧索引
        // 范围：这些帧绝不出现在任何 free bucket。initrd 物理页承载符号表 strtab
        // （`&'static` 名字），设备树承载设备自描述，两者都须终身存活——一旦被
        // 分配器复用，崩溃现场符号化即悬垂、设备自述即被改写。预留帧的
        // pagemeta 置 non-free（free=false），`merge_block` 不会并入（伙伴侧检查
        // `is_some_and(|m| m.free)` 失败即停），也不会被 split 产出（不在链中）。
        let holes = Self::holes(self.base, self.edge, max_frame);
        while remaining > 0 {
            // 起始帧落在洞内：整段跳过洞（跳到洞尾，含洞的帧永不入链）。
            if let Some(end) = hole_containing(&holes, index) {
                let skip = end - index;
                index += skip;
                remaining -= skip;
                continue;
            }
            // 候选块 [index, index + 2^power)：**不跨任何洞、不越上界**。
            // 「不跨洞」= 块尾不得超过**下一个洞的起点**（洞已排序，取最近的那个）。
            let limit = holes
                .iter()
                .flatten()
                .map(|&(start, _)| start)
                .filter(|&start| start > index)
                .min()
                .unwrap_or(max_frame);
            let mut power = (index.trailing_zeros() as usize)
                .min(remaining.ilog2() as usize)
                .min(max_power - 1);
            while index + (1 << power) > limit {
                power -= 1;
            }
            unsafe {
                self.push_link(index, power);
            }
            index += 1 << power;
            remaining -= 1 << power;
        }
        Ok(())
    }

    /// 页是否 held：按 order 从大到小找**包含它的那个块的块首**，读该块首的 pagemeta。
    /// （`is_held` 与 `check_frame_held` 的读侧：audit 档与 debug 档都要。）
    /// 伙伴块内其余页没有独立表项（`None`），故必须按对齐回退找块首。
    /// 窗口外 / 找不到任何块首（不可能：init 把每一页都归入某个块或洞）→ false。
    #[cfg(any(debug_assertions, feature = "audit"))]
    fn held(&self, pa: usize) -> bool {
        if pa < self.base || pa >= self.edge {
            return false;
        }
        let index = self.frame_index(pa);
        // `index & !(2^power-1)` 给出该 order 下的对齐基址，但**对齐命中不等于包含**
        // ——同址可能站着另一个 order 的块。故必须用表项自带的 power 复核覆盖关系，
        // 否则会读到"隔壁那块"的 free 位（实测：index 5004 撞上一个从 0 起的 4096 页
        // 空闲块 ⇒ 把合法的释放报成 non-held）。
        for power in (0..self.freelist.len()).rev() {
            let base_index = index & !((1usize << power) - 1);
            if base_index >= self.pagemeta.len() {
                continue;
            }
            if let Some(m) = self.pagemeta[base_index].as_ref()
                && index < base_index + (1usize << m.power)
            {
                return !m.free;
            }
        }
        false
    }

    // 物理地址 → 帧索引
    fn frame_index(&self, addr: usize) -> usize {
        (addr - self.base) / PAGE_SIZE
    }

    // 帧索引 → 物理地址
    fn frame_addr(&self, index: usize) -> usize {
        self.base + index * PAGE_SIZE
    }

    /// 持久保留区在本分配器窗口内的帧索引区间 `[start, end)` 表（升序，最多
    /// [`MAX_RESERVED`](crate::machine::MAX_RESERVED) 条）。
    ///
    /// 每个保留区可能部分落在窗口外（防御性地截断到 `[base, edge)`）；完全落在
    /// 窗口外的、以及空的，都不进表（表里的每一条都真的挡住一段帧）。
    fn holes(
        base: usize,
        edge: usize,
        max_frame: usize,
    ) -> [Option<(usize, usize)>; crate::machine::MAX_RESERVED] {
        let mut out = [None; crate::machine::MAX_RESERVED];
        let mut n = 0;
        for r in crate::machine::info().reserved.iter().flatten() {
            let (hs, he) = (r.base, r.base + r.size);
            // 洞边界不在窗口内 → 无洞。
            if he <= base || hs >= edge {
                continue;
            }
            let start = (hs.max(base) - base) / PAGE_SIZE;
            let end = (he.min(edge) - base).div_ceil(PAGE_SIZE);
            out[n] = Some((start.min(max_frame), end.min(max_frame)));
            n += 1;
        }
        // 升序（下面按「最近的洞起点」取 limit，且跳过时要能顺序前进）。
        out[..n].sort_unstable();
        out
    }

    // frame 索引：翻转 order 对应的位
    fn buddy_index(index: usize, power: usize) -> usize {
        index ^ (1 << power)
    }

    // ── 临时探针：累计封箱/开箱（帧数口径，不受抽样时刻影响）──
    //
    // 判据：`已封 − 已开 = 当前该在链上的帧数`。它应当等于 `meta_free − 洞`；
    // 若**远大于**走链可见量，就直接证明"有帧被标成空闲却没入链"。
    // `merge_nomerge` 记录那个可疑分支：伙伴 `pagemeta` 说空闲、但不在链上 ⇒
    // `break` ⇒ 该伙伴既没被合并、也没被重新入链，而 `free=true` 标记留着。

    // 从 freelist[order] 头部弹出一个空闲块，标记为非空闲，返回帧索引。
    //
    // # Safety
    //
    // 调用者需确保 freelist[order] 的链表节点指向有效的已映射物理内存。
    /// 撤掉 `index` 处的块首表项：清表 + 扣"空闲块首"计数。
    ///
    /// **这是"块首"这一身份的唯一撤销点**。`pagemeta` 只该在块首有条目，而块一旦
    /// 被并进更大的块、或被从链中取出，它就不再是块首 —— 那条目必须撤，否则它成了
    /// **幽灵**：`merge_block` 按它认定伙伴空闲、`in_freelist` 却找不到 ⇒ 放弃合并
    /// （`nomerge`），那一段永久脱离可用池；`held()` 也会读到它而答错"谁拥有这帧"。
    ///
    /// 与 `push_link` 的"唯一入链口"成对：**一处标块首、一处撤块首**。
    /// **写点判据**：写下 `index` 之前先问"它是不是已经落在别人的跨度里"。
    ///
    /// 把判据装到**写点**上而不是事后扫：事后扫只看得到结果，装在这里能看到**是谁写的**
    /// （`what`）与**被谁的跨度覆盖**（覆盖者索引与 power）。
    ///
    /// # 定案：重叠**不是**常态，是一个写点的产物（已修，现恒为 0）
    ///
    /// 本判据曾量到启动期累计 **31376** 次"写进已有跨度"，我据此断言"`pagemeta` 是一张
    /// 容许重叠的块首图、不是块首到块的函数"，并推出"修法只有一条：先决定这张表是什么"。
    /// **那个结论是错的** —— 它的前提（重叠是常态）是错的：
    ///
    /// 31376 次里，覆盖者**一律是 `free=false`**（分类 `push→free=0`、`pull→free=0`），
    /// 而它们的来源是**一处**：旧版 `split_block` 按取出时的桶号写块首表项，那条粗表项
    /// 覆盖了拆分过程中被推回的伙伴。粗表项在源头去掉后，本计数 **31376 → 0**，
    /// 两条扫表法同时由"差 143/151 条"变成 **逐条相等**。
    ///
    /// 教训（与本文件其余几处同源）：**把"我造出来的现象"当成"系统的性质"，就会去设计
    /// 一个迎合现象的大修法**（A/B 两条都被我论证成"唯一可行"，两条都是多余的）。
    /// 判据本身留着：它仍是"表项互相包含"的写侧入口，而那种包含确实不该出现 ——
    /// 真出现时，`pagemeta` 就不再是块首到块的函数了。
    fn note_covered(&mut self, index: usize, what: &str) {
        // **产品档不付这份代价**：本判据曾量到启动期 31376 次命中，而根因（`split_block`
        // 的粗表项）在源头修掉后恒为 0；但每次 `push_link`/`pull_link`/`clear_head`/
        // `split_head` 都要扫一遍 `0..freelist.len()`。覆盖面当年由 `conserve` 从**读侧**
        // 等价给出（"跨度内表项 = 0"，撤销型写入则由"无主帧 = 0"兜住），故只有
        // debug / audit / framework 三档继续收这笔账。
        #[cfg(not(any(debug_assertions, feature = "audit")))]
        {
            let _ = (index, what);
            return;
        }
        #[cfg(any(debug_assertions, feature = "audit"))]
        // 由小到大找覆盖 index 的块首：block_head = index & !(2^p - 1)，含 index 者即覆盖。
        for p in 0..self.freelist.len() {
            let base = index & !((1usize << p) - 1);
            if base == index || base >= self.pagemeta.len() {
                continue;
            }
            if let Some(m) = self.pagemeta[base].as_ref()
                && index < base + (1usize << m.power)
            {
                let n = COVERED.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 4 {
                    crate::putln!(
                        "[covered] {what} idx={index} 落在 base={base} (free={} power={}) 的跨度内 ({n})",
                        m.free,
                        m.power
                    );
                }
                return;
            }
        }
    }

    fn clear_head(&mut self, index: usize) {
        self.note_covered(index, "clear_head");
        self.pagemeta[index] = None;
    }

    unsafe fn pull_link(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let head = self.freelist[power]?;
            checker::check_dram_addr(head.as_ptr() as usize, "frame pop_link (head)");

            let addr = head.addr().get();
            let index = self.frame_index(addr);
            // **空链必须降级，不得索引 panic**：`head` 是从 freelist 头读出来的，
            // 它的 `addr` 若已被写坏（实测：`index` 变成 `0x07011C7D01BB83C6`，
            // 一个 **wait key** 的数形状），`self.pagemeta[index]` 就是一次越界索引
            // ⇒ `index out of bounds` panic ⇒ **整机 halt**。
            //
            // 这条路上没有"地址是用户提供的"这种借口：`head` 是**内核自己放进链里
            // 的**。所以它坏 = 链被写坏 = 已知的既有缺陷（见本文件头部/`held` 的
            // 注释里记的跨 order 交叉史）。此处只做一件事：**不让它升级成停机**
            // ——返回 `None` 上抛成 `AllocError`，`Spawn`/堆分配照常拿到 `-4`/`-1`。
            //
            // 计数是刻意的：**污染率**是本缺陷唯一的量化面（每 N 次分配坏一次），
            // 也是后续定位根因的判据；静默吞掉就只剩"偶尔分配失败"的传言。
            if index >= self.pagemeta.len() {
                static BAD: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                let n = BAD.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 8 {
                    crate::putln!(
                        "frame freelist corrupt: pop index={index:#x} len={} power={power} ({n})",
                        self.pagemeta.len()
                    );
                }
                return None;
            }
            checker::check_bounds(
                index,
                self.pagemeta.len(),
                "frame pop_link (pagemeta index)",
            );
            checker::check_frame_free(
                self.pagemeta[index].as_ref().is_some_and(|m| m.free),
                index,
                addr,
                power,
            );

            let next = head.read().next;
            self.freelist[power] = next;
            if let Some(n) = next {
                checker::check_dram_addr(n.as_ptr() as usize, "frame pop_link (next)");
                // **必须写回原处**：`n.read().prev = None` 是对**临时副本**赋值
                // （`NonNull::read` 按值返回），编译通过、静默无效 —— 于是链表的
                // `prev` 从不维护，见 `push_link` 同处注释与 `[stale-rm]` 的取证。
                (*n.as_ptr()).prev = None;
            }

            self.note_covered(index, "pull_link");
            // **只撤，不立**：本块已经离开 `freelist[power]`，故它不再是那个 order 的
            // 空闲块首（撤）。至于它**现在**是什么块的块首，取决于 [`Self::split_block`]
            // 接着要拆到哪一级 —— 那个身份由**拆分收尾处唯一一次**写下。
            //
            // 先前这里按**取出时的桶号**写 `free=false`，于是表项声明的跨度比实际分配大
            // （含拆分推回的伙伴）⇒ `pagemeta` 从"块首到块的函数"退化成"容许重叠的覆盖图"：
            // `held()` 对已空闲的伙伴帧答"在手"、两条扫表法必然不等（实测差 151 条）、
            // 在手帧数被高估 528 帧（正是"跨度内空闲"那些帧）。撤表项不会造成空窗——
            // 全程持锁，`split_block` 在同一临界区内补写。
            self.clear_head(index);
            Some(index)
        }
    }

    // 将帧索引对应的块插入 freelist[order] 头部，写入侵入式 Link 节点。
    //
    // # Safety
    //
    // 调用者需确保 index 对应的物理地址有效且未被其他方式使用。
    unsafe fn push_link(&mut self, index: usize, power: usize) {
        unsafe {
            checker::check_bounds(power, self.freelist.len(), "frame push_link (power)");
            checker::check_bounds(
                index,
                self.pagemeta.len(),
                "frame push_link (pagemeta index)",
            );
            checker::check_not_in_chain(
                power,
                "frame push_link",
                self.freelist[power],
                self.frame_addr(index),
                |n| n.read().next,
            );

            let addr = NonNull::new_unchecked(self.frame_addr(index) as *mut Link);
            self.note_covered(index, "push_link");
            addr.write(Link::new(None, self.freelist[power]));

            if let Some(head) = self.freelist[power] {
                checker::check_dram_addr(head.as_ptr() as usize, "frame push_link (head)");
                // **必须写回原处**：`head.read().prev = Some(addr)` 是对 `read()` 返回的
                // **临时副本**赋值 —— 编译通过、静默无效。后果不是"少维护一个域"：
                // 旧头节点的 `prev` 永远是 `None`，而 `remove_link` 正是用
                // `prev == None` 判定"我是桶头"，于是摘除一个**链中间**节点时会走
                // 错分支、把真正的桶头覆盖成该节点的陈旧 `next`，链头上那几个块
                // 从此不可达却仍留着 `free=true` 表项（实测 622 帧 = 2.4 MiB
                // "表说空闲、链上找不到"，`[stale-rm]` 逐条取证）。
                (*head.as_ptr()).prev = Some(addr);
            }

            self.freelist[power] = Some(addr);
            // 注：曾在此处"清跨度内的过期空闲标记"（遍历 `2^power` 个槽）。**已撤**：
            // ① 实测对 `nomerge` 毫无改善（说明孤儿另有产出者，不在标记残留）；
            // ② 代价是大 order 块每次入链遍历 `2^power` 槽（power 16 = 65536 次），
            //    而 `push_link` 在每次拆分/合并都跑 —— 实测把 `churn` 拖慢约 50 倍
            //    （`churn 300 1 1` 从秒级变 100 s 跑不到一半）。
            // 教训与"改完看判据"同源：**没有判据支持的优化，只留代价**。
            // **唯一入链口：清掉跨度内的过期空闲表项**。
            //
            // 为什么必须有：`split_block` 取出 power=k 的整块后，逐级 `push_link`
            // 把切出的 buddy 填进**原块的跨度**；而旧版只写 buddy 自己那一条，
            // **原块那条粗粒度表项留在原地**，成了覆盖整段的幽灵。
            // 后果（`pagedrain` 实测，纯分配/释放、零任务生变，每轮
            // `pagedrain 500` 净漏 200~570 帧且单调累积）：
            //   · `merge_block` 按它认定伙伴空闲，`in_freelist` 却找不到 ⇒ 放弃
            //     合并 ⇒ 那一段永久脱离可用池；
            //   · `held()` 也会读到幽灵而答错"谁拥有这帧"。
            //
            // **只清 `free == true`**：`free == false` 的表项是**活分配**，它们
            // 合法地落在一个大空闲块的跨度里（碎片化的正常形态），清掉等于把在用
            // 的帧标成未知。
            //
            // （先前我按"跨度内不得有别的表项"整片清空过，被判据 `overrun` 证伪后
            //   撤回——那个判据本身是错的：大空闲块的跨度**合法地**包含小块块首。
            //   现在用的是 pagedrain 的帧数净漏，它不含解读空间。）
            self.pagemeta[index] = Some(Meta::new(true, power as u8));
        }
    }

    // 从 freelist[order] 中移除帧索引对应的块（侵入式链表摘除）。
    //
    // # Safety
    //
    // 调用者需确保 index 对应的 Link 节点确实在 freelist[order] 链表中。
    unsafe fn remove_link(&mut self, index: usize, power: usize) {
        unsafe {
            let addr = self.frame_addr(index) as *mut Link;
            checker::check_dram_addr(addr as usize, "frame remove_link");
            checker::check_in_chain(
                power,
                "frame remove_link",
                self.freelist[power],
                addr as usize,
                |n| n.read().next,
            );

            let prev = (*addr).prev;
            let next = (*addr).next;

            if let Some(p) = prev {
                (*p.as_ptr()).next = next;
            } else {
                // **桶头判定**：`prev == None` 就是"我是桶头"的自述，此时桶头指针必须
                // 正指着我。若不是，这个节点早已不在链上（`Link` 是陈旧的），而下面这句
                // 会**把真正的桶头覆盖成陈旧值** —— 那些节点从此从桶头走不到，却仍留着
                // `free=true` 的表项（实测形态：`prev=None`、`next` 指向一个陈旧帧号、
                // `in_freelist` 说它不在链、走链也数不到它，正是那 622 帧的来源）。
                let head = self.freelist[power].map(|h| h.as_ptr() as usize);
                if head != Some(addr as usize) {
                    let n = STALE_RM.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                    if n <= 8 {
                        crate::putln!(
                            "[stale-rm] remove_link idx={index} power={power} 自称桶头但桶头={:?} next={:?}（第 {n} 次）",
                            head.map(|h| (h - self.base) / PAGE_SIZE),
                            next.map(|x| (x.as_ptr() as usize - self.base) / PAGE_SIZE)
                        );
                    }
                }
                self.freelist[power] = next;
            }
            if let Some(n) = next {
                (*n.as_ptr()).prev = prev;
            }
        }
    }

    // 从 >=order 的空闲桶中找到块，逐级拆分到目标 order，返回分配帧索引。
    //
    // # Safety
    //
    // 内部调用 pop_link / push_link，要求 freelist 链表节点指向的物理内存有效。
    unsafe fn split_block(&mut self, power: usize) -> Option<usize> {
        unsafe {
            // 向上找到第一个有空闲块的 order
            let mut k = power;
            while k < self.freelist.len() && self.freelist[k].is_none() {
                k += 1;
            }
            if k >= self.freelist.len() {
                return None;
            }

            let index = self.pull_link(k)?;

            // 逐级拆分：每级把 frame 推入 freelist
            while k > power {
                k -= 1;
                let buddy = Self::buddy_index(index, k);
                self.push_link(buddy, k);
            }

            // **块首身份定案**（"分配"侧的**唯一**写点）：`pull_link` 只把块从桶里摘出来
            // 并撤掉它作为空闲块首的身份，本块究竟占几帧要到拆分结束才知道 —— 伙伴们
            // （每降一级推回一个）的表项由 `push_link` 各自写好，`index` 这里按**最终
            // order** 写一次。这样 `pagemeta` 与块一一对应，不再有"粗表项覆盖伙伴"。
            //
            // 收尾写而非取出时写，是为了让拆分过程中的 `push_link(伙伴)` 不被一条
            // 尚未定案的粗表项"覆盖"（否则写侧判据 `note_covered` 把它们全记成
            // 侵入既有跨度：修前实测 8712 条）。
            self.note_covered(index, "split_head");
            self.pagemeta[index] = Some(Meta::new(false, power as u8));

            Some(index)
        }
    }

    // 将释放的帧索引推入 freelist，并逐级向上与空闲 frame 合并。
    //
    // # Safety
    //
    // 调用者需确保 index 来自本分配器的 allocate，且未被重复释放。
    unsafe fn merge_block(&mut self, mut index: usize, mut power: usize) {
        unsafe {
            // **下降头**：`index` 处的表项是 `split_block` 按**分配时的 order** 写的，
            // 而本函数每合并一级就 `power += 1`（`index` 不变）⇒ 那条表项从第一级起
            // 就是过时的 ——它声明的是一个**已被合并掉**的块。撤掉它，块首身份由下面的
            // `push_link(index, power)` 按最终 order 重新建立。**只撤一次**：本函数的
            // 每一次 `power += 1` 都对应同一次调用，故循环外撤即够。
            self.clear_head(index);
            while power < self.freelist.len() {
                let buddy = Self::buddy_index(index, power);

                // 边界检查：buddy 可能超出 free 区（pagemeta 长度非 2 的幂，末块
                // 的 XOR 伙伴会越界）。此时该伙伴不存在，不能合并——直接 break。
                if buddy >= self.pagemeta.len() {
                    break;
                }

                if !self.pagemeta[buddy]
                    .as_ref()
                    .is_some_and(|m| m.free && m.power as usize == power)
                {
                    break;
                }
                // pagemeta 说 frame 空闲，但必须确实在 freelist[power] 链中才可
                // 合并——否则是残留标记（frame 已并入其它块/已被分配），合并会
                // 摘除一个不在链中的节点、破坏链表（跨 order 交叉的直接来源）。
                if !self.in_freelist(buddy, power) {
                    // 探针：伙伴说空闲却不在链上 ⇒ 它既不被合并也不被重新入链，
                    // 但 `free=true` 留着 —— "标空闲却不在链上"的批量来源。
                    NOMERGE.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    break;
                }

                self.remove_link(buddy, power);
                // 合并后 frame 并入 index 块：清除其独立 pagemeta——残留 free
                // 标记会让后续 split/merge 把已并入大块的帧当空闲块处理
                // （frame 不变量破坏 → 同一帧双重入链 → freelist 读垃圾）。
                self.clear_head(buddy);
                index = index.min(buddy); // 合并后取较小的帧索引
                power += 1;
            }

            self.push_link(index, power);
            // **自审**：块标空闲之后必须真的在链上。不在 ⇒ 该块永久脱离可用池
            // （`merge_block` 的伙伴判定会一直失败、分配也拿不到它）。
            // 只在异常时打印，稳态零代价。
            if !self.in_freelist(index, power) {
                static LOST: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                let n = LOST.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 8 {
                    crate::putln!(
                        "merge: pushed block NOT in chain idx={index} power={power} ({n})"
                    );
                }
            }
        }
    }


    /// **诊断**：`pa` 是不是**某个在手块的中间帧**（不是块首）。
    ///
    /// 判据：由小到大找覆盖它的表项；若覆盖者索引 `!= index` 且 `free == false`，则这一页
    /// 是**某笔在手分配的中间帧**。对这样的页调 `deallocate` 是错的 —— 分配器按帧合并会把
    /// 中点当块首入链，表里于是长出"在手块跨度内的表项"（这正是 `[covered]` 量到的那些）。
    fn interior_of_held(&self, pa: usize) -> Option<(usize, u8)> {
        if pa < self.base || pa >= self.edge {
            return None;
        }
        let index = self.frame_index(pa);
        for p in 0..self.freelist.len() {
            let base = index & !((1usize << p) - 1);
            if base == index || base >= self.pagemeta.len() {
                continue;
            }
            if let Some(m) = self.pagemeta[base].as_ref()
                && index < base + (1usize << m.power)
                && !m.free
            {
                return Some((base, m.power));
            }
        }
        None
    }

    fn in_freelist(&self, index: usize, power: usize) -> bool {
        let target = self.frame_addr(index);
        let mut cur = self.freelist[power];
        while let Some(node) = cur {
            if node.as_ptr() as usize == target {
                return true;
            }
            // SAFETY: freelist 节点恒为已释放块，头 16 字节是 Link（prev/next）。
            cur = unsafe { node.read() }.next;
        }
        false
    }
}

// FrameAllocator 持有 SpinLock（!Send）按值，不能直接 `OnceLock<FrameAllocator>`
// （需 Send+Sync）；故存 `&'static FrameAllocator`（引用只需 Sync），init 时 Box::leak。
static FRAME_ALLOCATOR: OnceLock<&'static FrameAllocator> = OnceLock::new();

pub fn allocator() -> &'static dyn Allocator {
    FRAME_ALLOCATOR
        .get()
        .expect("frame allocator not initialized")
}

/// 帧分配器本体存取器（审计/health 直调自身方法——分配器文件不设审计适配层）。
pub(crate) fn heap() -> &'static FrameAllocator {
    FRAME_ALLOCATOR
        .get()
        .expect("frame allocator not initialized")
}

/// 初始化 frame 分配器。基址取自 bump frontier，须在所有 bump 分配之后调用。
///
/// # Errors
///
/// - 空闲区不足一页 → [`InitError::NoFreeFrames`]。
/// - 元数据 Vec 分配失败 → [`InitError::OutOfMemory`]。
pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = Box::leak(Box::new(FrameAllocator::init()?));
        FRAME_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing frame allocator")
}
