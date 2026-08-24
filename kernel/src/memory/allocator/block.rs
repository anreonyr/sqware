// 块分配器 — per-node 池 + 泵（pump）过境路由（segregated free list，单链表侵入式）
//
// 块堆全动态：**池不占静态区段**，页全部向 frame 分配器借（prime）、用空归还（drain）——
// 无 half/free 配比概念，池页占用 ≈ 当前需求（峰值可回吐，不活泄漏）。块 512 字节以下
// 由本层服务（hybrid 按 size ≤ PAGE/2 路由）；4096B 整页块已退役给 frame（order0）。
//
// 页生命周期：
//   prime  — 缺块时 frame::allocate 借 1 页：簿记表写 (owner<<12|used=1)，块整页
//            拆链入 freepool（块从页首 +0 起，无页内开销），返回首块；pool.pages += 1。
//   drain  — 页用空（used==0）时从 freepool 摘除本页全部块，帧归还**前**清簿记表项
//            （表项必须与帧生命周期同步——帧复用后不可残留旧主），frame::deallocate 归还，
//            pool.pages -= 1。**锁内逐块摘链 O(链长)**。
//   own    — 物理地址 → 归属池 id：簿记表查无主/主。deallocate 路由
//            home==me → push（本地）；否则 feed（进归属池 pump，异地归还）。
//
// 簿记表（外置，页内零开销——4096 页被 8B 页头破坏 2 的幂整除性会浪费 1024B/2048B
// 类 25%~50% 容量；记账移出后 2048B 可排 2 块、8B 可排 512 块满装）：
//   每页一条 Meta（owner/power/used 三字段，不做位打包，可读性优先）：
//   owner=归属池（None=无主）、power=页的 size class（drain 摘链定位）、used=在册块数。
//   idx = ((pa & !(PAGE-1)) - meta_base) >> PAGE_SHIFT。
//   无 MAGIC：own 两层防护 = 范围检查（区外 → None）+ owner=None（无主）。全部表
//   访问（读/写/复合 RMW）自锁（tally，Level::Tally）——跨核串行，无数据竞争；
//   portal 已无锁（无锁模式分派见 portal.rs），此锁是簿记表现任闸门。
//
// 分配策略（arena 迟滞）：
//   spare[power] — 每 size class 保留 1 个空闲页（used==0）不归还：push 归零时本 class
//   无保留页 → 本页补位；已有 → 本页 drain。pull 弹块若属于 spare 页 → 该页重新在用，
//   资格取消（spare 置 None）。平峰谷抖动，避免每页即还即借。
//
// 结构镜像 frame.rs——自上而下一条脊柱：
//   公共对象 BlockAllocator（池集合 + 簿记表，等价 FrameAllocator）
//       → Allocator 实现（直接在公共对象上）
//       → 每节点 BlockInner（锁壳，等价 FrameAllocator 的每节点一份）
//       → BlockInner 内状态 Pool（等价 FrameInner）
//       → 自由助手 → 静态实例 → allocator()/init()。
//
// 命名：pool(池) pump(泵) pull/push(池内拉/推) feed/suck(泵口喂/抽) prime(引水入泵/
// 借页) drain(排干/还页) own(归属)——4 字母动词成族；spare(备页) pages(持页数) 记账。
//
// 调试：校验/流水收容在 allocator::fence::checker（frame.rs 同用）——pull/push 本体
// 只留单行钩子；钩子恒编译、release 空体零开销；断言仅 debug 构建生效（见该模块）。
//
// 不变·硬（贴结构）：
//   - 块只进归属池的 freelist：feed 只入 pump，suck 是唯一转 push 的路径；
//   - used==0 ⇔ 本页全部块已 push 归位（跨节点 feed 的块不进泵不递减，归零瞬间
//     泵无残留）——drain 摘链安全的前提，见 drain；
//   - 簿记表自锁（tally）：own/inc_used/dec_used 单锁内原子；prime/drain 持 inner 调
//     frame（锁序 inner→frame→tally，从不反向；hybrid 路由保证 frame 永不回问 block）；
//   - 拓扑（块区）建成后只读；锁序 = pull/suck 先 pump 后 inner（摘空再归位），
//     feed 仅 pump，push/prime/drain 仅 inner——无环。

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
/// 块层最大 size class（≤ 半页：多块页，恒有页头；4096B 整页块已退役给 frame）。
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

    /// used +1（u16 饱和——满装 512 块远不到上限，不应溢出）。
    fn inc_used(self) -> Self {
        Self {
            used: self.used.saturating_add(1),
            ..self
        }
    }

    /// used -1（饱和）；返回 (新项, 是否归零)。
    fn dec_used(self) -> (Self, bool) {
        let used = self.used.saturating_sub(1);
        (Self { used, ..self }, used == 0)
    }
}

/// 簿记表（tally）：覆盖 free 区的页级账目，每页一条 `Meta`。
///
/// 下标语义：`idx(pa) = ((pa & !(PAGE-1)) - base) >> PAGE_SHIFT`——页对齐后从
/// free 区基址起算的页号。**不做 `Index` trait**：`Index::index` 须返回 `&Meta`，
/// 与表内存的共享可写（写路径仅持 `&self`）冲突、别名 UB；本表语义是范围检查
/// + 拷贝读。**全部表访问（含 RMW 复合步）自锁**（`lock`，Level::Tally）——跨核
/// 串行、无数据竞争；portal 已无锁（无锁模式分派见 portal.rs），此为簿记表
/// 的现任闸门。复合步（`inc_used`/`dec_used`）必须在单锁内完成读改写，否则
/// 锁间留丢更新窗口（旧 meta_get/meta_put 两段式在 portal 锁内曾靠外层串行）。
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

    /// 读表项（page 为本池持页，idx 必在表内）。
    fn read(&self, page: usize) -> Meta {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行。
        unsafe { self.cells.add(idx).read() }
    }

    /// 按下标读表项（clear 扫表用；idx 已由调用方保证 < len）。
    fn read_idx(&self, idx: usize) -> Meta {
        let _g = self.lock.lock();
        // SAFETY: idx < len 已由调用方保证；lock 串行。
        unsafe { self.cells.add(idx).read() }
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

    /// 复合读改写：used +1（**单锁内**完成读→改→写——旧 meta_get/meta_put 两段式
    /// 在锁间有丢更新窗口，此处杜绝）。
    fn inc_used(&self, page: usize) -> Meta {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行，RMW 原子。
        unsafe {
            let mut m = self.cells.add(idx).read();
            m.used = m.used.saturating_add(1);
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

// ── 过境驿站（pump）：同 tear 时代的结构不变 ──

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
/// 定位原语 own 直接查表（替代旧区段二分 pool_of——全动态下池无区段，归属记在
/// 表里随页走；页内零开销，4096 页满装）。
pub(crate) struct BlockAllocator {
    blocks: &'static [BlockInner],
    /// 簿记表（'static 共享；访问串行见 Tally 注释）。
    tally: &'static Tally,
}

impl BlockAllocator {
    /// 物理地址 → 归属池 id（deallocate 路由前提）。查簿记表：表项有主 →
    /// Some(owner)；区外或无主 → None（调用方静默丢弃，沿用旧 pool_of 语义）。
    fn own(&self, pa: usize) -> Option<usize> {
        self.tally.owner_of(pa)
    }

    /// 构建块分配器：按核数建池集合 + bump 分配簿记表。**不划区段**——池从 0
    /// 页起，页全部经 prime 向 frame 借（frame::init 之后首次分配即可借，自举
    /// 无碍：block::init 与 frame::init 同跑在 bump 后端，池首借发生在门户后端
    /// 切 hybrid 之后）。
    ///
    /// 必须在 `main` 早期调用恰好一次（经 [`init`]），bump 后端下执行——池元数据
    /// 经 bump 分配，不会重入本锁；且须在 frame::init 之前。
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

        // audit: 完整性框架装配——Banker 覆盖 free 区全页（借还由 frame 记账，
        // block 不重复 debit）；Ledger 按经验上限预留（全 free 区×每页块数不可行，
        // 见 fence::ledger::Ledger 注释；真实负载峰值远低于 512K）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        {
            crate::memory::allocator::fence::banker::BANKER.init(m.free.base, m.free.size / PAGE_SIZE);
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
        // 防御：block 直连（不经 hybrid）时 size > 半页的请求拒绝；正常路径
        // hybrid 已按 size ≤ PAGE/2 路由，不会到达。
        if power > MAX_POWER || layout.align() > (1usize << power) {
            return Err(AllocError);
        }
        let me = machine::hart_id();
        let pool = &self.blocks[me];
        let addr = pool.pull(power).ok_or(AllocError)?;
        // audit: 整块毒化（未初始化读现行）+ 活块入账（重复入账=块级双发现行；
        // KernelHeap 且 slack≥8 时 mark 内部写对齐 slack canary）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        {
            crate::memory::allocator::fence::poison(addr, 1usize << power);
            let caller: usize;
            // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
            unsafe { core::arch::asm!("mv {}, ra", out(reg) caller) };
            crate::memory::allocator::fence::ledger::LEDGER.mark(
                addr,
                layout.size(),
                caller,
                crate::memory::allocator::fence::ledger::OwnerKind::KernelHeap,
            );
        }
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
        // 归属路由：own 读页头（MAGIC 不符 = 非块内存 → 静默丢弃，非本层所有）。
        let Some(home) = self.own(pa) else { return };
        // audit: 注销账目（无账=双 free/悬垂、canary=越界、尺寸=错幂现行）+ 本体毒化
        // 复写（头 8B 随后被 freelist 头插覆盖，其余保持毒化——UAF 读数变 0xCD）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        {
            crate::memory::allocator::fence::ledger::LEDGER.unmark(pa, layout.size());
            crate::memory::allocator::fence::poison(pa, 1usize << power);
        }
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
    fn prime(&self, inner: &mut Pool, power: usize) -> Result<NonNull<u8>, AllocError> {
        // 借 1 页（order0）。audit: Banker debit 已由 frame::allocate 完成——页来自
        // frame 区且借出即 debit；drain 归还时 counterpart credit 同样由 frame 侧。
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let page = frame::allocator()
            .allocate(layout)
            .map_err(|_| AllocError)?;
        let base = page.as_ptr() as *mut u8 as usize;
        checker::check_dram_addr(base, "block prime (frame page)");

        // 簿记表：owner=本池、power=本类、used=1；块从页首 +0 起整页拆链（页内零开销，满装）
        self.meta_put(base, Meta::new(self.id, power));
        // 块数 = 一页能容纳的块数（除法，勿写成移位——曾误作 PAGE_SIZE<<power，
        // 越界狂写砸烂邻页导致启动崩溃）。
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
            inner.pages += 1;
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

        // 3. 归还 frame（audit: Banker credit + outstanding 递减由 frame 侧完成）
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        unsafe {
            frame::allocator().deallocate(NonNull::new_unchecked(page as *mut u8).cast(), layout);
        }
        inner.pages -= 1;
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
            // audit: 整页无活跃账目（used-counter 记账正确性检查；与 freepool
            // 内容无关——块都在链上正常周转）。
            #[cfg(all(debug_assertions, feature = "audit"))]
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

    /// 清空（关机）：suck 后由 flush_all 调用，归还全部空闲页。扫簿记表——
    /// 本池 owned 且 used==0 的页逐页 drain（页自含 power，摘链按表项定位）。
    /// spare 页同在归还之列——空闲即还，与"保留资格"无关；随后 spare 全清。
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
    /// 持页总数（prime +1 / drain -1）。audit 口径（原 torn_pages）与此审计断言。
    pages: usize,
    /// 每 size class 保留的空闲页（迟滞）；见模块头"分配策略"。
    spare: [Option<usize>; MAX_POWER + 1],
}

impl Pool {
    fn new() -> Self {
        Self {
            freepool: Vec::new(),
            pages: 0,
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

fn heap() -> &'static BlockAllocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

/// 停机前冲洗全部池：抽干 pump 归位过境块 + 清空空闲页还 frame，
/// 之后帧基线断言才成立。
pub(crate) fn flush() {
    for pool in heap().blocks {
        pool.suck();
        pool.clear();
    }
}

// ── 适配层（hybrid/audit 调用，接口零改动）──

/// audit: 地址是否为某池持有的块内存（簿记表归属判定；替代旧区段包含判定——
/// 全动态下池无区段，表项即权威）。未初始化返回 false。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) fn pool_includes(pa: usize) -> bool {
    heap().own(pa).is_some()
}

/// audit: 全池持页总数（prime 借出未还；关机基线断言与 Banker 交叉核对用）。
/// 池持有页已计入 frame outstanding——关机比较须剔除（见 fence::audit::check_baseline）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) fn held_pages() -> usize {
    heap().blocks.iter().map(|b| b.pool.lock().pages).sum()
}

pub fn allocator() -> &'static dyn Allocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

// ── 初始化 ──

/// 初始化块分配器（OnceLock 顶层单例装配；池构建见 [`BlockAllocator::init`]）。
///
/// 必须在 `main` 早期调用恰好一次（经 `allocator::init`），bump 后端下执行；且须在
/// frame::init 之前——池元数据先于 frame base 定址。
///
/// # Errors
///
/// 构建失败（`BlockAllocator::init` 的 [`InitError`]）/ 重复初始化 → [`InitError`]。
pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = BlockAllocator::init()?;
        BLOCK_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing block allocator")
}
