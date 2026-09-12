use crate::memory::PAGE_SIZE;
use core::ptr::NonNull;
use erra::ResultExt;

use alloc::{
    alloc::{AllocError, Allocator},
    boxed::Box,
    vec::Vec,
};

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

}

/// `remove_link` 里"自称桶头、桶头却不是我"的次数（陈旧 `Link` 导致桶头被覆盖）。
static STALE_RM: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

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
            #[cfg(feature = "framework")]
            {
                let a = addr as usize;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame alloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            // 取出的帧就来自空闲链本身——"哪些帧空闲"的唯一真相是 freelist，
            // pagemeta 的 free 位是它的派生视图，没有第二份每页位图要对。
            super::statistics::record_frame_take(index, power);
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
            #[cfg(feature = "framework")]
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
            // 归还入总量账（`occupied` 减一）。pagemeta 是"这帧在不在手"的唯一真相，
            // 总量账只是水位；合并在下一句里做。
            super::statistics::record_frame_give(index);
            frame.merge_block(index, power);
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
        // 每帧类目表与 pagemeta 同批（都在"所有 bump 分配完成"之前），尺寸按本步的
        // 暂估帧数 —— 它 ≥ 第二步收缩后的真实帧数，故索引永不出界。
        super::statistics::install_frame_kinds(max_frame)?;

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

    // 从 freelist[order] 头部弹出一个空闲块，标记为非空闲，返回帧索引。
    //
    // # Safety
    //
    // 调用者需确保 freelist[order] 的链表节点指向有效的已映射物理内存。
    fn clear_head(&mut self, index: usize) {
        self.pagemeta[index] = None;
    }

    unsafe fn pull_link(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let head = self.freelist[power]?;

            let addr = head.addr().get();
            let index = self.frame_index(addr);
            // **空链必须降级，不得索引 panic**：`head` 是从 freelist 头读出来的，
            // 它的 `addr` 若已被写坏（实测：`index` 变成 `0x07011C7D01BB83C6`，
            // 一个 **wait key** 的数形状），`self.pagemeta[index]` 就是一次越界索引
            // ⇒ `index out of bounds` panic ⇒ **整机 halt**。
            //
            // 这条路上没有"地址是用户提供的"这种借口：`head` 是**内核自己放进链里
            // 的**。所以它坏 = 链被写坏 = 已知的既有缺陷（见本文件头部记的跨 order
            // 交叉史）。此处只做一件事：**不让它升级成停机**
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

            let next = head.read().next;
            self.freelist[power] = next;
            if let Some(n) = next {
                // **必须写回原处**：`n.read().prev = None` 是对**临时副本**赋值
                // （`NonNull::read` 按值返回），编译通过、静默无效 —— 于是链表的
                // `prev` 从不维护，见 `push_link` 同处注释与 `[stale-rm]` 的取证。
                (*n.as_ptr()).prev = None;
            }

            // **只撤，不立**：本块已经离开 `freelist[power]`，故它不再是那个 order 的
            // 空闲块首（撤）。至于它**现在**是什么块的块首，取决于 [`Self::split_block`]
            // 接着要拆到哪一级 —— 那个身份由**拆分收尾处唯一一次**写下。
            //
            // 先前这里按**取出时的桶号**写 `free=false`，于是表项声明的跨度比实际分配大
            // （含拆分推回的伙伴）⇒ `pagemeta` 从"块首到块的函数"退化成"容许重叠的覆盖图"：
            // 按表项求和会把已空闲的伙伴帧算成"在手"、两条扫表法必然不等（实测差 151 条）、
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
            let addr = NonNull::new_unchecked(self.frame_addr(index) as *mut Link);
            addr.write(Link::new(None, self.freelist[power]));

            if let Some(head) = self.freelist[power] {
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
            //   · 按表项反查"谁拥有这帧"也会读到幽灵而答错。
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
            // 尚未定案的粗表项"覆盖"（否则表项会互相包含
            // 侵入既有跨度：修前实测 8712 条）。
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
                    // 伙伴说空闲却不在链上 ⇒ 它既不会被合并也不会被重新入链，但
                    // `free=true` 的标记留着 —— 正是"标空闲却不在链上"的来源。
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
