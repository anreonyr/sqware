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

    /// 池水位三元组：`(freelist 链上帧数, pagemeta 说"在手"帧数, 区总帧数)`。
    ///
    /// # 为什么是两份读法并排返回
    ///
    /// 先前所有「泄漏速率」都是拿 **freelist 走链**当判据的，而它与 `pagemeta`
    /// 记账对不上（走链说"放进去了"，`free` 却在跌）。两者在同一把锁下取，**在
    /// 同一次读数里自洽**——不等就说明走链不是真相，先前基于它的结论全部作废。
    ///
    /// 真相在 `pagemeta`：分配时写 `free=false`、释放时写回 `true`，与 `held()`
    /// 同一份表（见 [`Self::is_held`] 的「唯一真相」）。故：
    ///
    /// ```text
    /// pagemeta 在手 = Σ 块首 free=false 的块页数
    /// pagemeta 空闲 = Σ 块首 free=true  的块页数
    /// 两者之和 = 总帧数 − 洞（洞在 init 时写 free=false 且永不出现在链上，
    ///           故恒不计入任何一边）
    /// ```
    ///
    /// **`pagemeta` 在手单调上涨 = 泄漏**——与 freelist 链的完整度无关。
    pub(crate) fn watermark(&self) -> (usize, usize, usize) {
        let g = self.inner.lock();
        let (walk, held, idle) = Self::tally(&g);
        (walk, held, idle + held)
    }

    /// **只按 `pagemeta` 数空闲帧**（完全不碰 freelist 链）。
    ///
    /// # 为什么要第三个独立读数
    ///
    /// 实测出现了一个无法同时成立的组合：`step − held ≈ 11579` 帧按账该空闲，
    /// 而**走链**只报 `walk=349` —— 可 guest 又跑了几百轮不 OOM。两者必有一个错。
    ///
    /// 本函数是**不看链**的独立答案：把 `pagemeta` 里所有 `free=true` 的块按
    /// `2^power` 加起来。它与 [`Self::tally`] 的 `idle` 是同一个算法，但**分开
    /// 暴露**，以便和走链的 `walk` 三者并列对质：
    ///
    /// * `meta_free ≈ walk` ⇒ 链是好的，池子真的空了；
    /// * `meta_free ≫ walk` ⇒ **链断了/漏了**（`next` 被写坏而提前终止，或块没入链），
    ///   池子其实还有内存——此时"OOM"是**记账缺陷**造成的假象。
    ///
    /// 注意 `meta_free` 是**求和**口径，碎片化时会**高估**（大空闲块的跨度里可能
    /// 合法地站着小块、被重复计入）。故判据用**量级**（`≫`），不做精确相等。
    pub(crate) fn meta_free_frames(&self) -> usize {
        let g = self.inner.lock();
        g.pagemeta
            .iter()
            .flatten()
            .filter(|m| m.free)
            .map(|m| 1usize << m.power)
            .sum()
    }

    /// 那个"伙伴说空闲但不在链上 ⇒ 放弃合并"分支被走了多少次（探针）。
    ///
    /// 该分支 `break` 后，那个伙伴**既没被合并、也没被重新入链**，而它的
    /// `free=true` 标记留着——正是"`pagemeta` 说空闲、链上却找不到"的来源。
    /// 这个数在 churn 下单调增长即为该机制成立的直接证据。
    pub(crate) fn nomerge_count() -> usize {
        NOMERGE.load(::core::sync::atomic::Ordering::Relaxed)
    }

    /// 守恒量读数（探针）：`(累计分配帧, 累计释放帧)`。
    ///
    /// 恒等式：**每一帧要么在手、要么空闲** ⇒ `累计分配 − 累计释放 = held + walk`。
    /// 这是唯一不依赖 freelist 走链、也不依赖 `pagemeta` 的第三口径——三者对不上
    /// 时，它指出到底是谁在说假话。
    pub(crate) fn frame_ledger() -> (usize, usize) {
        use ::core::sync::atomic::Ordering;
        (
            TAKE_FRAMES.load(Ordering::Relaxed),
            GIVE_FRAMES.load(Ordering::Relaxed),
        )
    }

    /// **链↔表一致性核对**：链上每个块，其 `pagemeta` 表项必须是
    /// `Some(free=true, power=桶号)`。
    ///
    /// # 为什么这条核对可信（与我前五个探针的区别）
    ///
    /// 前五个探针（`overrun` / `census` / `frnet` / `merge_census` / `reachability`）
    /// 都**缺少自洽性约束**，所以总能产出一个看似有信息量的数，而我就照着讲，
    /// 随后被自己的数据推翻。本条不同：它是**逐块的两处状态对照**，两边都直接读
    /// 自权威结构（链节点 vs `pagemeta` 表项），不经过任何我自己的累加——
    /// 没有"分项之和 vs 总数"这类可漂移的口径。
    ///
    /// 返回 `(检查的块数, 不一致块数, 头 3 个不一致样本 (帧索引, 桶号, 表项))`。
    /// **临时诊断入口**：见 `FrameInner::scan_disagree`。
    /// 丢帧的中间帧释放 / 判据把块首认成中间帧 的次数。
    pub(crate) fn interior_split() -> (usize, usize) {
        use ::core::sync::atomic::Ordering;
        (
            crate::memory::allocator::fence::checker::LOSSY_FREES.load(Ordering::Relaxed),
            crate::memory::allocator::fence::checker::HEAD_ALIASED.load(Ordering::Relaxed),
        )
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

    /// 覆盖写入的**分类**读数：`[push→free, push→held, pull→free, pull→held, clear→free, 其它]`。
    pub(crate) fn covered_breakdown() -> [usize; 6] {
        let mut out = [0usize; 6];
        for (i, a) in COVERED_BY.iter().enumerate() {
            out[i] = a.load(::core::sync::atomic::Ordering::Relaxed);
        }
        out
    }


    /// **诊断入口**：见 `FrameInner::interior_of_held`。
    pub(crate) fn interior_of_held(&self, pa: usize) -> Option<(usize, u8)> {
        self.inner.lock().interior_of_held(pa)
    }

    /// 帧索引 → 物理地址（诊断用）。
    pub(crate) fn frame_addr_of(&self, index: usize) -> usize {
        self.inner.lock().frame_addr(index)
    }

    pub(crate) fn scan_disagree(&self) -> (usize, usize, usize, (usize, u8)) {
        self.inner.lock().scan_disagree()
    }

    pub(crate) fn chain_meta_mismatch(&self) -> (usize, usize, [(usize, usize, u8); 3]) {
        let g = self.inner.lock();
        let mut checked = 0usize;
        let mut bad = 0usize;
        // `u8` 编码：0 = 无表项，1 = free=false，2 = power 不符，3 = 地址出池。
        let mut sample = [(0usize, 0usize, 0u8); 3];
        for (order, head) in g.freelist.iter().enumerate() {
            let mut cur = *head;
            let mut budget = g.pagemeta.len() + 1;
            while let Some(node) = cur {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                let addr = node.as_ptr() as usize;
                // **不 break —— 记违规并继续走**。
                //
                // 旧版这三处都是 `break`：一遇到"表里没有它"就**丢掉整条链的余下部分**，
                // 于是那条节点以及它之后的一切都不计入 `bad`。实测正是这么漏掉的 ——
                // 逐 order 扫表比对时 `p=0` 出现"链=1、表=0"（链上有个节点它的索引没有
                // 条目），而本函数报 `mismatch=0`：它不是判对了，是**根本没走到**。
                // 判据的盲区比判据的错更坏：错会响，盲区只会沉默。
                let outside = addr < g.base || addr >= g.edge;
                let i = if outside {
                    0
                } else {
                    (addr - g.base) / PAGE_SIZE
                };
                checked += 1;
                let code = if outside || i >= g.pagemeta.len() {
                    3u8 // 节点地址不在池内 → 链被写坏
                } else {
                    match g.pagemeta[i].as_ref() {
                        None => 0u8,
                        Some(m) if !m.free => 1u8,
                        Some(m) if m.power as usize != order => 2u8,
                        Some(_) => 255u8, // 一致
                    }
                };
                if code != 255 {
                    if bad < 3 {
                        sample[bad] = (i, order, code);
                    }
                    bad += 1;
                }
                // SAFETY: freelist 节点恒为空闲块，头 16 字节是 Link。
                cur = unsafe { node.read() }.next;
            }
        }
        (checked, bad, sample)
    }


    /// **反向核对：`pagemeta` 里每条 `free=true` 表项，其块是否真在 `freelist[power]` 链上。**
    ///
    /// # 这是上一个检查缺的那一半
    ///
    /// `chain_meta_mismatch` 只验"链上每块的表项正确"（正向）。孤儿表项——不在任何
    /// 链上、却写着 `free=true` 的那些——从这个缺口**整片漏掉**。而 `walk`（503 帧，
    /// 13 个块）与 `meta`（11918 帧）的背离，唯一自洽的解释就是**大批孤儿表项**。
    ///
    /// 返回 `(自由表项数, 不在链上者数, 头 3 个样本 (帧索引, power))`。
    pub(crate) fn free_entry_orphans(&self) -> (usize, usize, [(usize, u8); 3]) {
        let g = self.inner.lock();
        let mut free_entries = 0usize;
        let mut orphans = 0usize;
        let mut sample = [(0usize, 0u8); 3];
        // 步进扫：表项只在块首，块占 `2^power` 帧。
        let mut cursor = 0usize;
        while cursor < g.pagemeta.len() {
            let Some(m) = g.pagemeta[cursor].as_ref() else {
                cursor += 1;
                continue;
            };
            let power = m.power as usize;
            let frames = 1usize << power;
            if m.free {
                free_entries += 1;
                if !g.in_freelist(cursor, power) {
                    if orphans < 3 {
                        sample[orphans] = (cursor, m.power);
                    }
                    orphans += 1;
                }
            }
            cursor += frames;
        }
        (free_entries, orphans, sample)
    }

    /// **地址集合比对**：`freelist[0]` 的全部节点地址 vs `pagemeta` 里 `free=true`
    /// 且 `power=0` 的表项地址。
    ///
    /// 目的：上一轮发现"`pagemeta` 有 ~7758 条空闲表项不在任何链上"，而按代码
    /// `push_link` 是唯一写这些表项的地方、且**无条件**入链——两者矛盾。本函数把
    /// 两边地址**原样打出来**，看差异集合的形状（整段连续 vs 散点），不做任何
    /// 自造口径。
    ///
    /// 返回 `(链上节点数, power=0 的空闲表项数, 链上头 3 个地址, 表项头 3 个地址)`。
    pub(crate) fn addr_sets(
        &self,
        power: usize,
    ) -> (usize, usize, [usize; 3], [usize; 3]) {
        let g = self.inner.lock();
        let mut chain = [0usize; 3];
        let mut cn = 0usize;
        let mut total_chain = 0usize;
        let mut cur = g.freelist.get(power).copied().flatten();
        let mut budget = g.pagemeta.len() + 1;
        while let Some(node) = cur {
            if budget == 0 {
                break;
            }
            budget -= 1;
            total_chain += 1;
            if cn < 3 {
                chain[cn] = node.as_ptr() as usize;
                cn += 1;
            }
            // SAFETY: freelist 节点恒为空闲块，头 16 字节是 Link。
            cur = unsafe { node.read() }.next;
        }
        let mut entries = [0usize; 3];
        let mut en = 0usize;
        let mut total_entries = 0usize;
        for (i, m) in g.pagemeta.iter().enumerate() {
            if let Some(m) = m.as_ref()
                && m.free
                && m.power as usize == power
            {
                total_entries += 1;
                if en < 3 {
                    entries[en] = g.base + i * PAGE_SIZE;
                    en += 1;
                }
            }
        }
        (total_chain, total_entries, chain, entries)
    }

    /// 拒绝按 power 的分布（探针）：`(power, REJ_META[p], REJ_CHAIN[p])`。
    pub(crate) fn reject_by_power(p: usize) -> (usize, usize, usize) {
        use ::core::sync::atomic::Ordering;
        let i = p.min(19);
        (
            i,
            REJ_META[i].load(Ordering::Relaxed),
            REJ_CHAIN[i].load(Ordering::Relaxed),
        )
    }

    /// `merge_block` 三道门各自的拒绝次数 + 成功合并次数（探针）。
    pub(crate) fn merge_census() -> (usize, usize, usize, usize) {
        use ::core::sync::atomic::Ordering;
        (
            MR_BOUND.load(Ordering::Relaxed),
            MR_META.load(Ordering::Relaxed),
            MR_CHAIN.load(Ordering::Relaxed),
            MR_OK.load(Ordering::Relaxed),
        )
    }

    /// 各 order 的 `(pagemeta 空闲块首条数, 链上实际块数)`（探针，固定 17 槽）。
    ///
    /// 两者不等 ⇒ 幽灵块：被标空闲却没进链。
    pub(crate) fn free_block_census(&self) -> [(usize, usize); 17] {
        let g = self.inner.lock();
        let mut out = [(0usize, 0usize); 17];
        for o in 0..17 {
            let chain = g
                .freelist
                .get(o)
                .map(|h| chain_len(*h, g.pagemeta.len() + 1))
                .unwrap_or(0);
            // **扫表**，不读 `META_FREE_BLOCKS`：那个计数器只在 `push_link` 加、只在
            // `clear_head` 减，而 `pull_link` 取出时**不减**（它直接覆写成 free=false）
            // ⇒ 计数器**单调虚高**，当口径用会得出"表比链多"的假象。判据只能直接问表。
            let mut heads = 0usize;
            let mut cursor = 0usize;
            while cursor < g.pagemeta.len() {
                let Some(m) = g.pagemeta[cursor].as_ref() else {
                    cursor += 1;
                    continue;
                };
                if m.free && m.power as usize == o {
                    heads += 1;
                }
                cursor += 1usize << m.power;
            }
            out[o] = (heads, chain);
        }
        out
    }

    /// freelist 累计收支（探针）：`(净帧数, 净块数)`，按入链时声明的 `power` 累加。
    pub(crate) fn freelist_ledger() -> (i64, i64) {
        use ::core::sync::atomic::Ordering;
        let net_frames =
            FR_PUSH.load(Ordering::Relaxed) as i64 - FR_PULL.load(Ordering::Relaxed) as i64;
        let net_blocks =
            BK_PUSH.load(Ordering::Relaxed) as i64 - BK_PULL.load(Ordering::Relaxed) as i64;
        (net_frames, net_blocks)
    }

    /// 走链超出步数上限的次数（探针）——`>0` 即**链成环**，也就是"卡死"的真身。
    pub(crate) fn chain_cycle_count() -> usize {
        CHAIN_CYCLE.load(::core::sync::atomic::Ordering::Relaxed)
    }

    /// 走链的**症状描述**：`(可见块数, 链尾是否为 None, 有没有指向区外的节点)`。
    ///
    /// # 为什么需要"症状"而不只是"计数"
    ///
    /// 实测走链总量掉到 113 而 `pagemeta` 说还有 ~12000 帧空闲 ⇒ **链断了**。
    /// 断链有三种形态，处置完全不同：
    ///
    /// * **提前终止**（某节点 `next = None` 却本不该是尾）：说明有节点被摘除时
    ///   没接上后续——查 `remove_link` / `merge_block`；
    /// * **指向区外**（`next` 落在 `[base, edge)` 之外或没对齐）：说明空闲块的
    ///   内存被**别人写了**（空闲块被复用/覆写）——查"谁动了空闲帧"；
    /// * **成环**（走不完）：说明同一块被入链两次——查 `push_link` 的重复入链。
    ///
    /// 返回的 `tail_none` 为真**不代表有问题**（正常的链就是以 None 结尾）；
    /// 判据是它与 `meta_free_frames` 的**量级差**——差得大就说明是前两种之一。
    pub(crate) fn chain_symptoms(&self) -> (usize, bool, usize) {
        let g = self.inner.lock();
        let lo = g.base;
        let hi = g.edge;
        let mut blocks = 0usize;
        let mut tail_none = false;
        let mut outside = 0usize;
        for (order, head) in g.freelist.iter().enumerate() {
            let mut cur = *head;
            // 防环护栏：最多走 `pagemeta.len() + 1` 步（合法块数不可能超过总帧数）。
            let mut budget = g.pagemeta.len() + 1;
            loop {
                let Some(node) = cur else {
                    tail_none = true;
                    break;
                };
                if budget == 0 {
                    break; // 成环
                }
                budget -= 1;
                blocks += 1;
                let addr = node.as_ptr() as usize;
                if addr < lo || addr >= hi || addr % PAGE_SIZE != 0 {
                    outside += 1;
                }
                // SAFETY: freelist 节点恒为空闲块，头 16 字节是 Link（prev/next）。
                cur = unsafe { node.read() }.next;
            }
            let _ = order;
        }
        (blocks, tail_none, outside)
    }

    /// 初始化刚结束时的 `pagemeta` 指纹：`(区总帧数, 表项条数, 步进覆盖帧数, 求和的空闲帧数)`。
    ///
    /// **为什么要读"出厂状态"**：churn 跑起来之后 `sum` 一路虚高、`overrun` 一路涨，
    /// 但那只能说明"运行中变坏了"。若出厂时 `表项条数 × 2^power` 就已经超过区总帧数
    /// （即洞被逐帧写成了表项），那"虚高"里有一大块是**先天**的，与 churn 无关——
    /// 判据必须先把这两部分分开，否则会把出厂状态当成运行期腐化去修。
    pub(crate) fn init_fingerprint(&self) -> (usize, usize, usize, usize) {
        let g = self.inner.lock();
        let frames = g.pagemeta.len();
        let entries = g.pagemeta.iter().flatten().count();
        let (_, _, idle_step) = Self::tally(&g);
        let mut idle_sum = 0usize;
        for m in g.pagemeta.iter().flatten() {
            if m.free {
                idle_sum += 1usize << m.power;
            }
        }
        (frames, entries, idle_step, idle_sum)
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

    /// `pagemeta` 逐表项**求和**的跨度数：`(持有帧数, 空闲帧数)`。
    /// # 它不是"真相"，是**上界**
    ///
    /// 先前我把它当真相、并据此断言"表项互相重叠 ⇒ 记账腐化"。**那个断言是错的**：
    /// buddy 分配器里，一个大的**空闲**块，其跨度内**合法地**站着若干小块的块首
    /// （碎片化的正常形态，它们不是这个空闲块的伙伴、不该合并）。求和会把小块
    /// 再算一遍，故 `和 ≥ 步进解` 恒成立，且差值就是碎片量。
    ///
    /// 判"漏没漏"的正确读数是 [`Self::tally`] 的**步进解**（它 `walk + held` 守恒），
    /// 不是这个和。本函数只作诊断对照保留。
    pub(crate) fn pagemeta_sum(&self) -> (usize, usize) {
        let g = self.inner.lock();
        let mut held = 0usize;
        let mut idle = 0usize;
        for m in g.pagemeta.iter().flatten() {
            let frames = 1usize << m.power;
            if m.free {
                idle += frames;
            } else {
                held += frames;
            }
        }
        (held, idle)
    }
}

static NOMERGE: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

/// ── 临时探针：freelist **累计收支**（零分配，只用原子）──
///
/// 前两版探针都在回收路径上按 `pagemeta.len()` 开 `Vec`，两次都把整机拖进退化态。
/// 这一版**完全不碰堆**。判据：`net_frames` 按**入链时声明的 `power`** 累加，
/// 与走链累加（`walk`，按**桶号**算）对质：
///   · 两者不等 ⇒ 块被放进了**不是它自己 order 的桶**（按桶算自然对不上）；
///   · 两者相等而都远小于 `meta` ⇒ 块确实没进链。
/// `merge_block` 三道门各自拒绝了多少次（探针，零堆分配）。
///
/// 判据：`pagedrain` 显示每个 order-0 块释放后都停在 order-0（`walk_delta` 恒为
/// +1），说明合并从未发生。这三道 `break` 里必有一道在拒绝——计数直接指认是哪道。
/// `merge_block` 拒绝时**按 power 分桶**计数（探针，17 槽定长、零堆分配）。
///
/// `REJ_META[p]` = 「pagemeta 说伙伴不空闲/order 不对」在 power=p 拒了几次；
/// `REJ_CHAIN[p]` = 「pagemeta 说空闲，但 `in_freelist` 找不到」在 power=p 拒了几次。
///
/// 判据：`meta=0`（总）已经排除了第一道门，故 `REJ_CHAIN` 是主因。看它集中在哪个
/// `p`，就能判断是"块从未挂上"还是"挂在别的 order 桶"——两者的 `p` 分布不同。
static REJ_META: [::core::sync::atomic::AtomicUsize; 20] =
    [const { ::core::sync::atomic::AtomicUsize::new(0) }; 20];
static REJ_CHAIN: [::core::sync::atomic::AtomicUsize; 20] =
    [const { ::core::sync::atomic::AtomicUsize::new(0) }; 20];

static MR_BOUND: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static MR_META: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static MR_CHAIN: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static MR_OK: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

/// `pagemeta` 里各 order 的**空闲块首**条数（探针，固定 17 槽、零堆分配）。
///
/// 判据：它必须等于 `freelist[order]` 的**实际链长**。两者不等 ⇒ 有块被标为空闲
/// 却没进链（幽灵块）—— 而且本计数**只由 `pagemeta` 写点驱动**，不经过任何
/// 遍历，故它自己不会"看不见"东西。
/// **写点判据**：写入一个**已被别条表项跨度覆盖**的索引的次数。
///
/// 块首之间不可能互相包含（每条表项声明"从本索引起、占 `2^power` 帧"），所以这个数
/// **必须恒为 0**。实测非零且有稳定样本 —— 见 `health/stress.rs::chain()` 的哨兵。
static COVERED: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

/// 覆盖写入的分类：`push|pull|clear` × 被覆盖者 `free=true|false`。
static COVERED_BY: [::core::sync::atomic::AtomicUsize; 6] =
    [const { ::core::sync::atomic::AtomicUsize::new(0) }; 6];

static META_FREE_BLOCKS: [::core::sync::atomic::AtomicUsize; 17] =
    [const { ::core::sync::atomic::AtomicUsize::new(0) }; 17];

static GIVE_FRAMES: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static TAKE_FRAMES: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static FR_PUSH: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static FR_PULL: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static BK_PUSH: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
static BK_PULL: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

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
/// 停止遍历并**记一笔**（`CHAIN_CYCLE`），让"链成环"从"卡死"变成"可读的事实"。
fn chain_len(mut n: Option<NonNull<Link>>, cap: usize) -> usize {
    let mut blocks = 0usize;
    let mut budget = cap;
    while let Some(node) = n {
        if budget == 0 {
            CHAIN_CYCLE.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
            break;
        }
        budget -= 1;
        blocks += 1;
        // SAFETY: freelist 节点恒为空闲块，头 16 字节是 Link（prev/next）。
        n = unsafe { node.read() }.next;
    }
    blocks
}

static CHAIN_CYCLE: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

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
            TAKE_FRAMES.fetch_add(1usize << power, ::core::sync::atomic::Ordering::Relaxed);
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
            GIVE_FRAMES.fetch_add(1usize << power, ::core::sync::atomic::Ordering::Relaxed);
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
    /// # 实测读数：这不是边界情况，是常态
    ///
    /// 启动期一次运行累计 **31376** 次"写进已有跨度"，且多次运行稳定 —— `pagemeta`
    /// **是一张容许重叠的块首图**，不是块首到块的函数。这正是两条扫表法差 143 条的根：
    /// 它们都建立在"表项互不重叠"这个**表并不满足**的假设上。
    ///
    /// 由此也解释了修法 A/B 为什么撞墙：A 想靠"块缩小时唯一化"消灭别名（但重叠是常态，
    /// 逐个撤无法穷尽）；B 想靠"这个索引有没有自己的表项"回答"在谁手里"（同样以不重叠
    /// 为前提）。
    ///
    /// 真修法只有一条：**先决定这张表是"块首到块的函数"还是"容许重叠的覆盖图"**，
    /// 然后让写侧与读侧同守那一个决定。现状是两种假设混用。
    fn note_covered(&mut self, index: usize, what: &str) {
        // 由小到大找覆盖 index 的块首：block_head = index & !(2^p - 1)，含 index 者即覆盖。
        for p in 0..self.freelist.len() {
            let base = index & !((1usize << p) - 1);
            if base == index || base >= self.pagemeta.len() {
                continue;
            }
            if let Some(m) = self.pagemeta[base].as_ref()
                && index < base + (1usize << m.power)
            {
                // 分类计数：**写入者 × 被覆盖者的 free 态**。直接上"写侧消解覆盖"很可能
                // 砸掉合法的拆分/合并写点，故先量分布 —— 如果绝大多数是"空闲被空闲覆盖"
                // （被切开的原块残留），那才是可安全消解的那部分。
                let slot = match (what, m.free) {
                    ("push_link", true) => 0,
                    ("push_link", false) => 1,
                    ("pull_link", true) => 2,
                    ("pull_link", false) => 3,
                    ("clear_head", true) => 4,
                    _ => 5,
                };
                COVERED_BY[slot].fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
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
        if let Some(old) = self.pagemeta[index].as_ref()
            && old.free
        {
            META_FREE_BLOCKS[(old.power as usize).min(16)]
                .fetch_sub(1, ::core::sync::atomic::Ordering::Relaxed);
        }
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
                n.read().prev = None;
            }

            FR_PULL.fetch_add(1usize << power, ::core::sync::atomic::Ordering::Relaxed);
            BK_PULL.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
            self.note_covered(index, "pull_link");
            self.clear_head(index);
            self.pagemeta[index] = Some(Meta::new(false, power as u8));
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
                head.read().prev = Some(addr);
            }

            self.freelist[power] = Some(addr);
            // 注：曾在此处"清跨度内的过期空闲标记"（遍历 `2^power` 个槽）。**已撤**：
            // ① 实测对 `nomerge` 毫无改善（说明孤儿另有产出者，不在标记残留）；
            // ② 代价是大 order 块每次入链遍历 `2^power` 槽（power 16 = 65536 次），
            //    而 `push_link` 在每次拆分/合并都跑 —— 实测把 `churn` 拖慢约 50 倍
            //    （`churn 300 1 1` 从秒级变 100 s 跑不到一半）。
            // 教训与"改完看判据"同源：**没有判据支持的优化，只留代价**。
            FR_PUSH.fetch_add(1usize << power, ::core::sync::atomic::Ordering::Relaxed);
            BK_PUSH.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
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
            META_FREE_BLOCKS[power.min(16)].fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
            self.pagemeta[index] = Some(Meta::new(true, power as u8));
            let end = (index + (1usize << power)).min(self.pagemeta.len());
            for slot in &mut self.pagemeta[index + 1..end] {
                if let Some(m) = slot.as_ref()
                    && m.free
                {
                    META_FREE_BLOCKS[(m.power as usize).min(16)]
                        .fetch_sub(1, ::core::sync::atomic::Ordering::Relaxed);
                    *slot = None;
                }
            }
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
            // **下降头**：`index` 处的表项是 `pull_link` 按当时的 `power` 写的，而本函数
            // 每合并一级就 `power += 1`（`index` 不变）⇒ 那条表项从第一级起就是过时的
            // ——它声明的是一个**已经不存在**的块。撤掉它，块首身份由下面的
            // `push_link(index, power)` 按最终 order 重新建立。**只撤一次**：本函数的
            // 每一次 `power += 1` 都对应同一次调用，故循环外撤即够。
            self.clear_head(index);
            while power < self.freelist.len() {
                let buddy = Self::buddy_index(index, power);

                // 边界检查：buddy 可能超出 free 区（pagemeta 长度非 2 的幂，末块
                // 的 XOR 伙伴会越界）。此时该伙伴不存在，不能合并——直接 break。
                if buddy >= self.pagemeta.len() {
                    MR_BOUND.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    break;
                }

                if !self.pagemeta[buddy]
                    .as_ref()
                    .is_some_and(|m| m.free && m.power as usize == power)
                {
                    MR_META.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    REJ_META[power.min(19)].fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    break;
                }
                // pagemeta 说 frame 空闲，但必须确实在 freelist[power] 链中才可
                // 合并——否则是残留标记（frame 已并入其它块/已被分配），合并会
                // 摘除一个不在链中的节点、破坏链表（跨 order 交叉的直接来源）。
                if !self.in_freelist(buddy, power) {
                    // 探针：伙伴说空闲却不在链上 ⇒ 它既不被合并也不被重新入链，
                    // 但 `free=true` 留着 —— "标空闲却不在链上"的批量来源。
                    NOMERGE.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    MR_CHAIN.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    REJ_CHAIN[power.min(19)].fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                    break;
                }

                self.remove_link(buddy, power);
                // 合并后 frame 并入 index 块：清除其独立 pagemeta——残留 free
                // 标记会让后续 split/merge 把已并入大块的帧当空闲块处理
                // （frame 不变量破坏 → 同一帧双重入链 → freelist 读垃圾）。
                self.clear_head(buddy);
                MR_OK.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
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

    /// 帧是否在 freelist[power] 链中（遍历核对）。
    ///
    /// merge 合并 frame 前调用：pagemeta 可能残留 free 标记（frame 已并入
    /// 其它块），链中核对可避免摘除不存在的节点——这是 frame 一致性修复的
    /// 本体，release 同样生效（不是纯调试防御）。
    /// **临时诊断**：两条独立扫表法对质 —— 步进法（`free_entry_orphans`）vs 逐条法。
    ///
    /// 判据是"两者必须相等"：`pagemeta` 里每条表项都声明自己是块首（占 `2^power` 帧），
    /// 所以 (a) 步进扫一次、每次跳 `2^power`，与 (b) 逐条数 `flatten()`，**必须给出同一个数**。
    /// 不等就意味着**有条目落在别人声明占用的跨度里** —— 那正是别名。
    ///
    /// 返回 `(步进法条数, 逐条法条数, 错位条数, (索引, power) 首个错位样本)`。
    fn scan_disagree(&self) -> (usize, usize, usize, (usize, u8)) {
        let mut stepped = 0usize;
        let mut cursor = 0usize;
        let mut first = (0usize, 0u8);
        while cursor < self.pagemeta.len() {
            let Some(m) = self.pagemeta[cursor].as_ref() else {
                cursor += 1;
                continue;
            };
            stepped += 1;
            cursor += 1usize << m.power;
        }
        let flat = self.pagemeta.iter().flatten().count();
        // 错位：块首索引必须按自身大小对齐（`index % 2^power == 0`）。
        let mut misaligned = 0usize;
        for (i, m) in self.pagemeta.iter().enumerate() {
            if let Some(x) = m.as_ref()
                && i & ((1usize << x.power) - 1) != 0
            {
                if misaligned == 0 {
                    first = (i, x.power);
                }
                misaligned += 1;
            }
        }
        (stepped, flat, misaligned, first)
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
