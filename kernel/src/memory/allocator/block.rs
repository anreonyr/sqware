// 块分配器 — per-node 池 + 泵（pump）过境路由（segregated free list，单链表侵入式）。
//
// 块堆全动态：池不占静态区段，页全部向 frame 借（prime）、用空归还（drain）；
// ≤ 半页的块由本层服务。
//
// 簿记表（tally）：free 区每页一条 Meta（owner=归属池 / power=size class / used=在册
// 块数），全部表访问自锁（Level::Tally）。idx = ((pa & !(PAGE-1)) - meta_base) >> PAGE_SHIFT。
//
// 分配策略（arena 迟滞）：spare[power] 每 size class 保留 1 个空闲页（used==0）不归还，
// 平峰谷抖动，避免每页即还即借。
//
// 命名：pool(池) pump(泵) pull/push(池内拉/推) feed/suck(泵口喂/抽) prime(借页)
// drain(还页) own(归属)；spare(备页) pages(持页数) 记账。
//
// 不变·硬（贴结构）：
//   - 块只进归属池的 freelist：feed 只入 pump，suck 是唯一转 push 的路径；
//   - used==0 ⇔ 本页全部块已 push 归位——drain 摘链安全的前提；
//   - 簿记表自锁（tally）：own/inc_used/dec_used 单锁内原子；prime/drain 持 inner 调
//     frame（锁序 inner→frame→tally，从不反向）；
//   - 拓扑（块区）建成后只读；锁序 = pull/suck 先 pump 后 inner，feed 仅 pump，
//     push/prime/drain 仅 inner——无环。

use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::vec::Vec;
use erra::ResultExt;

use super::fence::checker;
use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::{
    lock::{Level, OnceLock, SpinLock},
    memory::allocator::{InitError, InitResult, bump, frame},
};

// ── 常量 ──

const MIN_POWER: usize = 3;
/// 块层最大 size class（≤ 半页：多块页，恒有页头）。
const MAX_POWER: usize = (PAGE_SIZE / 2).ilog2() as usize;
/// 页偏移位宽（簿记表下标换算用）。
const PAGE_SHIFT: usize = PAGE_SIZE.ilog2() as usize;

// ── 簿记表辅助（访问全部自锁（tally），调用方不必另行持锁）──

/// 簿记表项（每页一条，4B/页——不做位打包，字段直读直写，可读性优先）：
///   owner — 归属池 id；None = 无主（帧自由/非块内存页）
///   power — 页所属 size class（drain 摘链按此定位 freepool）
///   used  — 在册块数（满装 512 块，u16 富余）
#[derive(Clone, Copy, PartialEq, Eq)]
struct Meta {
    owner: Option<u8>,
    power: u8,
    used: u16,
}

impl Meta {
    const fn free() -> Self {
        Self {
            owner: None,
            power: 0,
            used: 0,
        }
    }

    const fn new(owner: usize, power: usize) -> Self {
        Self {
            owner: Some(owner as u8),
            power: power as u8,
            used: 1,
        }
    }

    fn owner(self) -> Option<usize> {
        self.owner.map(|o| o as usize)
    }

    fn used(self) -> u16 {
        self.used
    }

    /// used +1（u16 饱和——满装 512 块远不到上限，不应溢出）。Tally 复合步调用。
    fn inc_used(self) -> Self {
        Self {
            used: self.used.saturating_add(1),
            ..self
        }
    }

    /// used -1（饱和）；返回 (新项, 是否归零)。Tally 复合步调用。
    fn dec_used(self) -> (Self, bool) {
        let used = self.used.saturating_sub(1);
        (Self { used, ..self }, used == 0)
    }
}

/// 簿记表（tally）：覆盖 free 区的页级账目，每页一条 `Meta`。
///
/// 下标语义：`idx(pa) = ((pa & !(PAGE-1)) - base) >> PAGE_SHIFT`。**不做 `Index` trait**：
/// `Index::index` 须返回 `&Meta`，与表内存的共享可写（写路径仅持 `&self`）冲突、别名 UB；
/// 本表语义是范围检查 + 拷贝读。全部表访问（含 RMW 复合步）自锁（Level::Tally）——
/// 复合步（`inc_used`/`dec_used`）必须在单锁内完成读改写。
struct Tally {
    /// free 区基址（表覆盖下界）。
    base: usize,
    /// 表数据（bump 分配，'static）。
    cells: *mut Meta,
    /// 表长（free 区页数）。
    len: usize,
    /// 串行锁（Level::Tally）。
    lock: SpinLock<()>,
}

// SAFETY: cells 指向 'static bump 内存；全部访问经 lock 串行——跨核无数据竞争。
// 声明 Send+Sync 使 `&'static Tally` 可在 BlockAllocator 与各 BlockInner 间共享、
// 顶层 OnceLock<BlockAllocator> 成立。
unsafe impl Send for Tally {}
unsafe impl Sync for Tally {}

impl Tally {
    fn new(base: usize, cells: *mut Meta, len: usize) -> Self {
        Self {
            base,
            cells,
            len,
            lock: SpinLock::new_level(Level::Tally, ()),
        }
    }

    /// 物理地址 → 表下标（页对齐、上下界检查）。区外/下溢 → None。
    fn idx(&self, pa: usize) -> Option<usize> {
        let page = pa & !(PAGE_SIZE - 1);
        let idx = page.checked_sub(self.base)? >> PAGE_SHIFT;
        (idx < self.len).then_some(idx)
    }

    /// 下标 → 帧地址（clear 扫表用）。
    fn frame_of(&self, idx: usize) -> usize {
        self.base + (idx << PAGE_SHIFT)
    }

    /// 表长（clear 扫表循环上界）。
    fn len(&self) -> usize {
        self.len
    }

    /// 物理地址 → 归属池 id（deallocate 路由前提）。两层防护：范围检查（`idx`
    /// 区外 → None）+ 表项无主（owner=None）。
    fn owner_of(&self, pa: usize) -> Option<usize> {
        let _g = self.lock.lock();
        // SAFETY: idx 通过上下界检查；lock 串行，读与写互斥。
        let m = unsafe { self.cells.add(self.idx(pa)?).read() };
        m.owner()
    }

    /// 按下标读表项（clear 扫表用；idx 已由调用方保证 < len）。
    fn read_idx(&self, idx: usize) -> Meta {
        let _g = self.lock.lock();
        // SAFETY: idx < len 已由调用方保证；lock 串行。
        unsafe { self.cells.add(idx).read() }
    }

    /// 收集全部「有主」页 PA — 审计/诊断专用（关机池页计数诊断）。
    ///
    /// 闭包在 tally 锁内逐项读取（O(表长)），对每条 owner.is_some() 的表项
    /// 反算 PA 推入 out。注意本函数持 tally 锁期间调用方不得持 frame 锁
    /// （持锁顺序 tally < frame，与 prime/drain 同向；见模块头锁序）。
    /// out 使用审计类分配器（check_baseline 调用方传入）——审计工具自扰豁免。
    /// 仅 audit（debug-gated）用。
    #[cfg(feature = "audit")]
    pub(crate) fn collect_owned_pa(&self, out: &mut Vec<usize, &'static dyn Allocator>) {
        let _g = self.lock.lock();
        for idx in 0..self.len {
            // SAFETY: idx < len 由循环保证；lock 串行。
            let m = unsafe { self.cells.add(idx).read() };
            if m.owner.is_some() {
                out.push(self.frame_of(idx));
            }
        }
    }

    /// 写表项。
    fn write(&self, page: usize, m: Meta) {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行。
        unsafe {
            self.cells.add(idx).write(m);
        }
    }

    /// 复合读改写：used +1。
    fn inc_used(&self, page: usize) -> Meta {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行，RMW 原子。
        unsafe {
            let mut m = self.cells.add(idx).read();
            m = m.inc_used();
            self.cells.add(idx).write(m);
            m
        }
    }

    /// 复合读改写：used -1（同上）；返回 (新项, 是否归零)。
    fn dec_used(&self, page: usize) -> (Meta, bool) {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行，RMW 原子。
        unsafe {
            let m = self.cells.add(idx).read();
            let (m, empty) = m.dec_used();
            self.cells.add(idx).write(m);
            (m, empty)
        }
    }
}

// ── 过境驿站（pump）──

/// 过境驿站（pump）一处：按 size class 分立的一条侵入式单向链表——块自身内存存
/// next（同 freepool 手法），跨节点释放时把块首 8 字节挂链。**零分配**：feed 常处
/// 于 deallocate 持锁上下文（own 持 tally、路由后持 pump），任何分配都会重入分配
/// 器锁（inner/tally/frame）——递归或死锁（lockdep，见 lock/depend）；suck 摘空
/// 整链（O(1) 换头）后于 pump 锁外逐块 push 归位。
struct Pump {
    head: Option<NonNull<u8>>,
    /// 在途块数（诊断/审计：feed 递增、suck 归零，恒等于链长）。
    len: usize,
}

impl Pump {
    const fn new() -> Self {
        Self { head: None, len: 0 }
    }

    /// 头插：把链头写入块首 8 字节（同 push 的 freepool 头插，块尺寸 ≥ 8B 恒够）。
    unsafe fn push(&mut self, ptr: NonNull<u8>) {
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>().write(self.head);
            self.head = Some(ptr);
            self.len += 1;
        }
    }

    /// 摘空整链（O(1)）并返回旧链头；len 归零。
    fn take(&mut self) -> Option<NonNull<u8>> {
        let head = self.head.take();
        self.len = 0;
        head
    }
}

// ── 公共对象：池集合 + 簿记表 —— 等价 Frame 的 FrameAllocator（无区段表）──

/// 块堆本体：每节点一个 BlockInner + 全局簿记表 Tally（覆盖 free 区全页）。
pub(crate) struct BlockAllocator {
    blocks: &'static [BlockInner],
    /// 簿记表（'static 共享；访问串行见 Tally 注释）。
    tally: &'static Tally,
}

impl BlockAllocator {
    /// 物理地址 → 归属池 id（deallocate 路由前提）。查簿记表：表项有主 →
    /// Some(owner）；区外或无主 → None（调用方静默丢弃，沿用旧 pool_of 语义）。
    pub(crate) fn own(&self, pa: usize) -> Option<usize> {
        self.tally.owner_of(pa)
    }

    /// 收集全部「有主」页 PA（任何池 owned；本块堆全部持有页）。
    /// 关机池页计数诊断用（audit::check_baseline ④）。
    #[cfg(feature = "audit")]
    pub(crate) fn collect_owned(&self, out: &mut Vec<usize, &'static dyn Allocator>) {
        self.tally.collect_owned_pa(out);
    }

    /// 构建块分配器：按核数建池集合 + bump 分配簿记表（池从 0 页起，页经 prime 向 frame 借）。
    ///
    /// 必须在任何堆分配之前调用恰好一次，且须在 frame 初始化之前。
    ///
    /// # Errors
    ///
    /// 元数据分配失败（bump 池耗尽） → [`InitError::OutOfMemory`]。
    fn init() -> Result<Self, InitError> {
        let nodes = machine::hart_count();
        assert!(nodes > 0, "block init: no harts");
        let m = machine::info();

        // 簿记表：free 区每页一条 Meta，全 free（无主）。
        let tally = {
            let meta_len = m.free.size.div_ceil(PAGE_SIZE);
            let meta = bump::allocator()
                .allocate(
                    core::alloc::Layout::from_size_align(
                        meta_len * core::mem::size_of::<Meta>(),
                        core::mem::align_of::<Meta>(),
                    )
                    .unwrap(),
                )
                .map_err(|_| InitError::OutOfMemory)?;
            let cells = meta.as_ptr() as *mut Meta; // 指向 NonNull<[u8]> 的 data 区
            unsafe {
                for i in 0..meta_len {
                    cells.add(i).write(Meta::free());
                }
            }
            Box::leak(Box::new(Tally::new(m.free.base, cells, meta_len)))
        };

        let mut pools = Vec::new();
        for i in 0..nodes {
            let mut pool = Pool::new();
            pool.init()?;
            pools.push(BlockInner::new(i, tally, pool));
        }

        // audit: 完整性框架装配（Banker + Ledger）。
        #[cfg(feature = "audit")]
        {
            crate::memory::allocator::fence::banker::BANKER
                .init(m.free.base, m.free.size / PAGE_SIZE);
            crate::memory::allocator::fence::ledger::LEDGER.init(512 * 1024);
        }

        Ok(BlockAllocator {
            blocks: Box::leak(pools.into_boxed_slice()),
            tally,
        })
    }
}

unsafe impl Allocator for BlockAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let power = layout
            .size()
            .max(1usize << MIN_POWER)
            .next_power_of_two()
            .ilog2() as usize;
        // 防御：size > 半页 / 对齐超块尺寸的请求拒绝。
        if power > MAX_POWER || layout.align() > (1usize << power) {
            return Err(AllocError);
        }
        let me = machine::hart_id();
        let pool = &self.blocks[me];
        let addr = pool.pull(power).ok_or(AllocError)?;
        // 护栏事件：活块入账（类别记账收在 fence：mark 默认 Persistent，打标
        // 分配器 relabel——本文件零类别词汇，见 fence 模块头解耦纪律）。
        super::fence::on_alloc(addr, layout.size(), super::fence::OwnerKind::KernelHeap);
        // SAFETY: pull 返回的地址必非零（分配器保证）。
        Ok(NonNull::slice_from_raw_parts(
            unsafe { NonNull::new_unchecked(addr as *mut u8) },
            1usize << power,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let power = layout
            .size()
            .max(1usize << MIN_POWER)
            .next_power_of_two()
            .ilog2() as usize;
        let pa = ptr.addr().get();
        // 归属路由：非块内存 → 静默丢弃。
        let Some(home) = self.own(pa) else { return };
        // 护栏事件：活块注销。
        super::fence::on_free(pa, layout.size(), super::fence::OwnerKind::KernelHeap);
        let me = machine::hart_id();
        let pool = &self.blocks[home];
        if home == me {
            pool.push(ptr, power);
        } else {
            pool.feed(ptr, power);
        }
    }
}

// ── 每节点池：锁壳 —— 等价 Frame 的 FrameAllocator 每节点一份 ──

pub(crate) struct BlockInner {
    /// 本池 id（= 核 id；prime 写表 owner 用）。
    id: usize,
    /// 簿记表（与 BlockAllocator 共享同一 Tally；used 记账经此）。
    tally: &'static Tally,
    pool: SpinLock<Pool>,                  // freepool + spare/pages 记账
    pump: [SpinLock<Pump>; MAX_POWER + 1], // 过境驿站：按 size class 分立；只收 feed，suck 抽空
}

impl BlockInner {
    fn new(id: usize, tally: &'static Tally, pool: Pool) -> BlockInner {
        BlockInner {
            id,
            tally,
            pool: SpinLock::new(pool),
            pump: core::array::from_fn(|_| SpinLock::new(Pump::new())),
        }
    }

    // ── 簿记表访问（自锁见 Tally；写路径仅此一处，读走 own/inc/dec）──

    /// 写表项（prime 入账 / drain 清账；tally 自锁串行）。
    fn meta_put(&self, page: usize, m: Meta) {
        self.tally.write(page, m);
    }

    // ── 池内核心：拉 / 推 / 借 / 还 ──

    /// 拉出一块：先 suck 归位过境块，再从 freelist 取；无则 prime 借页拆链。
    fn pull(&self, power: usize) -> Option<usize> {
        self.suck();
        let mut g = self.pool.lock();
        let inner = &mut *g;
        if let Some(head) = inner.freepool[power] {
            checker::check_dram_addr(head.as_ptr() as usize, "block pull (freepool head)");
            // spare 资格取消：保留页被重新在用 → 释放保留名额
            let page = head.as_ptr() as usize & !(PAGE_SIZE - 1);
            if inner.spare[power] == Some(page) {
                inner.spare[power] = None;
            }
            let next = unsafe { head.cast::<Option<NonNull<u8>>>().read() };
            inner.freepool[power] = next;
            // used 记账：复合 RMW 单锁内完成（tally 自锁；见 Tally::inc_used）。
            self.tally.inc_used(page);
            checker::log_alloc(head.as_ptr() as usize, power);
            return Some(head.as_ptr() as usize);
        }
        // 无现成块：向 frame 借页拆入链，首块即本次分配结果
        let first = self.prime(inner, power).ok()?;
        Some(first.as_ptr() as usize)
    }

    /// 借一页拆块入链（arena 扩展：池无自有区段，页即向 frame 借）。
    /// 调用方须已持本池 inner 锁（pull 内调用）。锁序：inner → frame（单向）。
    /// 页类别 = Pool（fence 打标分配器——自由周转，关机只做诊断计数，
    /// 不参与归零检查；本文件对类别词汇仅此一处）。
    fn prime(&self, inner: &mut Pool, power: usize) -> Result<NonNull<u8>, AllocError> {
        // 借 1 页（order0）。
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let page = crate::tag!(
            Pool,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| AllocError)?
        );
        super::statistics::record_block_take(self.id);
        let base = page.as_ptr() as *mut u8 as usize;
        checker::check_dram_addr(base, "block prime (frame page)");

        // 簿记表：owner=本池、power=本类、used=1；块从页首 +0 起整页拆链（页内零开销，满装）
        self.meta_put(base, Meta::new(self.id, power));
        // 块数 = 一页能容纳的块数（除法，勿写成移位）。
        let block_nums = PAGE_SIZE >> power;
        unsafe {
            {
                let block_size = 1usize << power;
                for i in 0..block_nums.saturating_sub(1) {
                    let this = base + i * block_size;
                    let next = base + (i + 1) * block_size;
                    NonNull::new_unchecked(this as *mut Option<NonNull<u8>>)
                        .write(Some(NonNull::new_unchecked(next as *mut u8)));
                }
                if block_nums > 0 {
                    NonNull::new_unchecked(
                        (base + (block_nums - 1) * block_size) as *mut Option<NonNull<u8>>,
                    )
                    .write(None);
                }
            };
            // 首块本次被分配：先入链（链头写给 next）再清首块 next，防整链随首块丢失。
            let first = NonNull::new_unchecked(base as *mut u8);
            inner.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
            first.cast::<Option<NonNull<u8>>>().write(None);
            Ok(first)
        }
    }

    /// 归一页：把本页全部块从 freepool[power] 摘除并归还 frame。
    /// 调用方须已持本池 inner 锁；前置：表项 used==0（全块 push 归位，pump 无残留，
    /// 见模块头不变·硬）。摘链 O(链长)。锁序：inner → frame（单向，同 prime）。
    fn drain(&self, inner: &mut Pool, power: usize, page: usize) {
        // 1. 摘除本页全部块（首块 next 覆盖前先读；重链其余块）
        let mut keep: Option<NonNull<u8>> = None;
        let mut head = inner.freepool[power];
        let mut n = 0usize;
        while let Some(node) = head {
            n += 1;
            if n > 1 << 16 {
                panic!("block allocator: drain[{power}] walk exceeded depth — cyclic chain?");
            }
            // SAFETY: 链中块均已在 freepool（free 状态），首字为 next 指针。
            let next = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
            let addr = node.as_ptr() as usize;
            if addr >= page && addr < page + PAGE_SIZE {
                // 本页块：摘除（不入新链）
            } else {
                // 其余块：重链（头插，保持 freepool 结构不变）
                unsafe {
                    node.cast::<Option<NonNull<u8>>>().write(keep);
                }
                keep = Some(node);
            }
            head = next;
        }
        inner.freepool[power] = keep;

        // 2. 清簿记项（**先于归还 frame**——表项与帧生命周期同步，帧复用后不可残留）
        self.meta_put(page, Meta::free());

        // 3. 归还 frame
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        unsafe {
            frame::allocator().deallocate(NonNull::new_unchecked(page as *mut u8).cast(), layout);
        }
        super::statistics::record_block_give(self.id);
    }

    /// 推回本池：写 freelist 链 + 递减表项计数；归零走 spare/drain 决策。
    fn push(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pool.lock();
        let inner = &mut *g;

        checker::check_not_in_chain(
            power,
            "block push",
            inner.freepool[power],
            ptr.as_ptr() as usize,
            |n| unsafe { n.cast::<Option<NonNull<u8>>>().read() },
        );

        // 头插
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
        }
        inner.freepool[power] = Some(ptr);
        checker::log_dealloc(ptr.as_ptr() as usize, power);

        // 递减表项计数；归零 → 本 class 无 spare 则补位（迟滞保留），有则归还本页。
        // used 记账：复合 RMW 单锁内完成（tally 自锁；见 Tally::dec_used）。
        let page = ptr.as_ptr() as usize & !(PAGE_SIZE - 1);
        let (_, empty) = self.tally.dec_used(page);
        if empty {
            // 整页无活跃账目。
            #[cfg(feature = "audit")]
            crate::memory::allocator::fence::audit::page_clear(page);
            if inner.spare[power].is_none() {
                inner.spare[power] = Some(page);
            } else {
                self.drain(inner, power, page);
            }
        }
    }

    // ── 泵：喂 / 抽（不变）──

    /// 喂入本池 pump：块在外地被释放时投递至此（送它回家）。只入驿站，绝不碰 inner；
    /// 块首 8 字节挂链（同 freepool 手法），**零分配**——本路径常处 deallocate 持锁
    /// 上下文（见 Pump 注释），任何分配都会重入分配器锁。
    fn feed(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pump[power].lock();
        // SAFETY: 块已被释放（调用方不再使用）；首 8 字节空闲可写，尺寸 ≥ 8B。
        unsafe { g.push(ptr) };
    }

    /// 抽干本池 pump：按 size class 逐链摘空（pump 锁内仅 O(1) 换头），摘出的链
    /// 在 pump 锁外逐块 push 归位（锁序 pump→放→inner；幂等）。读 next 必须先于
    /// push——push 会把 freepool 链头写进块首字、覆盖 next。
    fn suck(&self) {
        for power in MIN_POWER..=MAX_POWER {
            let head = {
                let mut g = self.pump[power].lock();
                g.take()
            };
            let mut this = head;
            let mut n = 0usize;
            while let Some(node) = this {
                // 护栏：同块双 feed 会让同一块两次入链（甚至自环）——深度越界即报
                // 错，避免抽空死循环（链被破坏时宁可 panic 也不要挂死）。
                n += 1;
                if n > 1 << 14 {
                    panic!(
                        "block allocator: pump[{power}] walk exceeded depth — cyclic chain (double feed?)"
                    );
                }
                // SAFETY: 链中块均已被释放、首字为 next 指针（feed 挂链专用），可读。
                let next = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
                self.push(node, power);
                this = next;
            }
        }
    }

    /// 清空（关机）：归还全部空闲页。扫簿记表——本池 owned 且 used==0 的页逐页
    /// drain（页自含 power，摘链按表项定位）。spare 页同在归还之列，随后全清。
    fn clear(&self) {
        let mut g = self.pool.lock();
        let inner = &mut *g;
        // 扫表：idx 遍历表全体；表项持页/无主两态在线（本池只拖自己的空闲页）。
        for idx in 0..self.tally.len() {
            let m = self.tally.read_idx(idx);
            if m.owner() == Some(self.id) && m.used() == 0 {
                let frame = self.tally.frame_of(idx);
                self.drain(inner, m.power as usize, frame);
            }
        }
        // 空闲页已全还（含 spare 页）；spare 若有残留引用即悬空，全清。
        inner.spare.iter_mut().for_each(|s| *s = None);
    }
}

// ── 池内状态：freepool + 持页数 + 备页 —— 等价 Frame 的 FrameInner ──

struct Pool {
    freepool: Vec<Option<NonNull<u8>>>,
    /// 每 size class 保留的空闲页（迟滞）；见模块头"分配策略"。
    spare: [Option<usize>; MAX_POWER + 1],
}

impl Pool {
    fn new() -> Self {
        Self {
            freepool: Vec::new(),
            spare: [None; MAX_POWER + 1],
        }
    }

    fn init(&mut self) -> Result<(), InitError> {
        self.freepool
            .try_reserve(MAX_POWER + 1)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freepool.resize_with(MAX_POWER + 1, || None);
        Ok(())
    }
}

// ── 静态实例 + 访问器 ──

static BLOCK_ALLOCATOR: OnceLock<BlockAllocator> = OnceLock::new();

/// 块堆本体存取器（审计/health 直调自身方法——分配器文件不设审计适配层）。
pub(crate) fn heap() -> &'static BlockAllocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

/// 冲洗全部池：抽干 pump 归位过境块 + 清空空闲页还 frame。
pub(crate) fn flush() {
    for pool in heap().blocks {
        pool.suck();
        pool.clear();
    }
}

pub fn allocator() -> &'static dyn Allocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

// ── 初始化 ──

/// 初始化块分配器：必须在任何堆分配之前调用恰好一次，且须在 frame 初始化之前。
pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = BlockAllocator::init()?;
        BLOCK_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing block allocator")
}
