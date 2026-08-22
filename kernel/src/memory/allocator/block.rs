// 块分配器 — per-node 池 + 泵（pump）过境路由（segregated free list，单链表侵入式）
//
// 块堆按节点分池：每个节点一个 Pool，池自带物理区段 [base, edge)；节点内多核共享
// 一把 inner 锁；块从池区段上撕页（tear）拆链——页来自自有区段，**不经过帧分配器**
// （帧层保持全局，只管栈/trap/页表页）。跨节点释放经泵路由：块在异地释放时 feed 进
// 其归属池的 pump 驿站，属主核下次 pull 前 suck 抽回归位——正确性不依赖任何调度策略
// （任务可自由迁移，内存子系统免疫调度行为）。驿站为按 size class 分立的侵入式单链
// （块首 8 字节挂 next，同 freepool 手法），喂/抽全程零分配——feed 常处持门户锁的
// deallocate 上下文，pump 若扩容分配会经 portal 自重入（lockdep 报警，见 Pump）。
//
// 结构镜像 frame.rs——自上而下一条脊柱：
//   公共对象 BlockAllocator（池集合 + 区段表，等价 FrameAllocator）
//       → Allocator 实现（直接在公共对象上）
//       → 每节点 BlockInner（锁壳，等价 FrameAllocator 的每节点一份）
//       → BlockInner 内状态 Pool（等价 FrameInner）
//       → 自由助手 → 静态实例 → allocator()/init()。
//
// 命名：pool(池) pump(泵) pull/push(池内拉/推) feed/suck(泵口喂/抽) tear(撕页)——全部
// 4 字母动词成族；base/edge 区段两端成对；pool_of/node_of 是定位原语。
//
// 不变·硬（贴结构）：
//   - 块只进归属池的 freelist：feed 只入 pump，suck 是唯一转 push 的路径；
//   - 页头 used 计数只在池 inner 锁内写（feed 拿不到 inner）；
//   - 拓扑（区段表）建成后只读；锁序 = pull/suck 先 pump 后 inner（摘空再归位），
//     feed 仅 pump，push/tear 仅 inner——无环。

use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::vec::Vec;
use erra::ResultExt;
use log::debug;

use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::putln;
use crate::{
    lock::{OnceLock, SpinLock},
    memory::allocator::{InitError, InitResult, bump},
};

// ── 常量 ──

const MIN_POWER: usize = 3;
const MAX_POWER: usize = PAGE_SIZE.ilog2() as usize;

// ── 公共对象：池集合 + 区段表 —— 等价 Frame 的 FrameAllocator ──

/// 过境驿站（pump）一处：按 size class 分立的一条侵入式单向链表——块自身内存存
/// next（同 freepool 手法），跨节点释放时把块首 8 字节挂链。**零分配**：feed 常处
/// 于 deallocate 持门户（portal）锁的上下文，任何分配都会经 portal 自重入
/// （spinlock 递归 → lockdep panic，见 lock/depend）；suck 摘空整链（O(1) 换头）
/// 后于 pump 锁外逐块 push 归位。
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

/// 区段：一段物理内存 → 池 id（按 base 有序，二分查询；init 后只读）。
struct Segment {
    base: usize,
    edge: usize,
    pool: usize,
}

/// 块堆本体：每节点一个 BlockInner + 按 base 有序的区段表（init 后一次装配，之后只读）。
/// 定位原语 pool_of 直接挂在对象上；核→节点映射见 `node_of`。
pub(crate) struct BlockAllocator {
    blocks: &'static [BlockInner],
    segments: &'static [Segment],
}

impl BlockAllocator {
    /// 页地址 → 池 id（区段覆盖判定；非常规堆内存返回 None）。
    fn pool_of(&self, pa: usize) -> Option<usize> {
        let segs = self.segments;
        // 二分：找最后一个 base <= pa 的段
        let idx = segs.partition_point(|s| s.base <= pa);
        if idx == 0 {
            return None;
        }
        let s = &segs[idx - 1];
        (pa < s.edge).then_some(s.pool)
    }

    /// 构建块分配器：按核数均分空闲区，给每核划一块池区段，建池集合 + 区段表
    /// （segments 按 base 升序，二分查询）。顶层单例装配见自由函数 [`init`]。
    ///
    /// 池总预算 = 空闲区一半（另一半留给 frame 区），均分到每池——每池大小随
    /// 机器内存自适应，不设编译期常量。单核退化 = 单池拿整个预算（原 POOL_SIZE
    /// 16 MiB 只是 128M 机器的特例值，此处自动算得更大）。
    ///
    /// 必须在 `main` 早期调用恰好一次（经 [`init`]），bump 后端下执行——池元数据
    /// 经 bump 分配，不会重入本锁；且须在 bump 所有元数据分配之后、frame base
    /// 计算之前：池区段划在 bump frontier 最前部，frame 从其后开始，两区不相交。
    ///
    /// # Errors
    ///
    /// 元数据分配失败 / 区段划走失败（含空闲区不足均分） → [`InitError::OutOfMemory`]。
    fn init() -> Result<Self, InitError> {
        let nodes = machine::hart_count();
        assert!(nodes > 0, "block init: no harts");
        // 预算：空闲区一半分给池(均分到每池)，另一半留给 frame 区。
        let m = machine::info();
        let per_pool_pages = m.free.size / 2 / nodes / PAGE_SIZE;
        if per_pool_pages < 1 {
            // 至少 1 块页
            return Err(InitError::OutOfMemory);
        }

        let mut pools = Vec::new();
        let mut segments = Vec::new();
        for i in 0..nodes {
            let layout = Layout::from_size_align(per_pool_pages * PAGE_SIZE, PAGE_SIZE).unwrap();
            let region = bump::allocator()
                .allocate(layout)
                .map_err(|_| InitError::OutOfMemory)?;
            // 区段 = 全部块页（unitmap 数组区已删，1/9 预留回归块区）。
            let base = region.as_ptr() as *const u8 as usize;
            let edge = base + per_pool_pages * PAGE_SIZE;

            let mut pool = Pool::new();
            pool.init(base)?;
            let blk = BlockInner::new(base, edge, pool);
            pools.push(blk);
            segments.push(Segment {
                base,
                edge,
                pool: i,
            });
        }

        // audit: 完整性框架装配——Banker 覆盖帧池全 free 区；Ledger 按块区页数预留容量。
        #[cfg(all(debug_assertions, feature = "audit"))]
        {
            let m = machine::info();
            let fbase = m.free.base;
            let fpages = m.free.size / PAGE_SIZE;
            crate::memory::integrity::BANKER.init(fbase, fpages);
            let pool_pages = per_pool_pages * nodes;
            crate::memory::integrity::LEDGER.init(pool_pages.saturating_mul(4));
        }

        Ok(BlockAllocator {
            blocks: Box::leak(pools.into_boxed_slice()),
            segments: Box::leak(segments.into_boxed_slice()),
        })
    }

    /// OOM 现场打印（block 分配失败路径调用）：分配点 + 本池撕页水位 + 该 power
    /// freepool 链长 + 全池 pump 在途块数（跨核滞留即泄漏的直接证据）+ portal
    /// 入口捕获的业务调用点。
    /// 调用点持门户锁（Portal→hybrid→block 链）——**零分配**、只用 try_lock +
    /// putln! 直写（同 dump_crash_site 纪律）；锁忙跳过不阻塞。
    fn dump_oom(&self, me: usize, power: usize, request: usize, caller: usize) {
        // 0. **先**捕获栈窗口（任何格式化打印之前）：putln!/fmt 的帧会占据栈顶
        // 并挤掉业务帧（上版教训）。失败发生在正常执行流，父链完好；范围限定
        // 镜像 .text（到 _rodata_start 为止），bss/数据值不会误收。
        let sp: usize;
        // SAFETY: 读栈指针无副作用。
        unsafe { core::arch::asm!("mv {}, sp", out(reg) sp) };
        unsafe extern "C" {
            static _rodata_start: u8;
        }
        let text_end = (&raw const _rodata_start).addr();
        let mut hits = [0usize; 12];
        let mut nh = 0usize;
        let hi = sp.saturating_add(0x4000);
        let mut a = sp & !7;
        while a < hi && nh < hits.len() {
            // SAFETY: 只读本执行流的栈区间（S 态恒等映射）。
            let w = unsafe { (a as *const usize).read_volatile() };
            if w >= 0x8020_0000 && w < text_end && w & 3 == 0 && (nh == 0 || hits[nh - 1] != w)
            {
                hits[nh] = w;
                nh += 1;
            }
            a += 8;
        }
        putln!("[block-oom] stack-scan text ({nh}):");
        for w in hits[..nh].iter() {
            putln!("[block-oom]   {w:#x}");
        }
        putln!(
            "[block-oom] hart {} request {request} B -> block power {power} ({} B) FAILED, caller {:#x}",
            machine::hart_id(),
            1usize << power,
            caller
        );
        let pool = &self.blocks[me];
        match pool.inner.try_lock() {
            Some(g) => {
                putln!(
                    "[block-oom] pool{me}: base {:#x} edge {:#x} cursor {:#x} (torn {} pages, {} bytes left)",
                    pool.base,
                    pool.edge,
                    g.cursor,
                    (g.cursor - pool.base) / crate::memory::PAGE_SIZE,
                    pool.edge.saturating_sub(g.cursor)
                );
                // 该 power freepool 链长（限深防环，同 push 护栏）。
                let mut depth = 0usize;
                let mut cur = g.freepool[power];
                while let Some(node) = cur {
                    depth += 1;
                    if depth > 1 << 14 {
                        putln!("[block-oom] freepool[{power}] walk EXCEEDED depth — cyclic?");
                        break;
                    }
                    // SAFETY: freepool 节点恒为已释放块，首 8B 是 next（同 push 遍历）。
                    cur = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
                }
                putln!("[block-oom] freepool[{power}] length {}", depth);
                // **全 class 链长普查**：归还进错 class（dealloc layout 与 alloc 不
                // 一致 / 跨类 double-free）会让失败 class 空、别的 class 藏着巨链。
                putln!("[block-oom] freepool chain lengths by power:");
                for (pw, head) in g.freepool.iter().enumerate() {
                    if head.is_none() {
                        continue;
                    }
                    let mut d = 0usize;
                    let mut node = *head;
                    while let Some(n) = node {
                        d += 1;
                        if d > 1 << 16 {
                            putln!("[block-oom]   power {pw}: walk EXCEEDED — cyclic?");
                            break;
                        }
                        // SAFETY: freepool 节点恒为已释放块，首 8B 是 next。
                        node = unsafe { n.cast::<Option<NonNull<u8>>>().read() };
                    }
                    putln!(
                        "[block-oom]   power {pw} ({} B): length {d}, head {:#x}",
                        1usize << pw,
                        head.unwrap().as_ptr() as usize
                    );
                }
                // 活块净值分布：15.8 MB 被哪些 size class 占用（分配+1/归还-1 对账）。
                // 全 power 7 = 小而均匀的 Arc/Vec 类泄漏；大 power 高 = 块级泄漏。
                putln!("[block-oom] live blocks by power:");
                let mut live_total = 0usize;
                for (pw, n) in g.live.iter().enumerate() {
                    if *n != 0 {
                        putln!(
                            "[block-oom]   power {pw} ({} B): {n} live ≈ {} B",
                            1usize << pw,
                            n.saturating_mul(1usize << pw)
                        );
                        live_total += n.saturating_mul(1usize << pw);
                    }
                }
                putln!("[block-oom]   live total ≈ {live_total} B of pool ({} B)",
                    pool.edge.saturating_sub(pool.base));
                putln!(
                    "[block-oom]   cumulative allocs {} / frees {} (delta {})",
                    g.allocs,
                    g.frees,
                    g.allocs.saturating_sub(g.frees)
                );
                drop(g);
            }
            None => putln!("[block-oom] pool{me} inner busy (skip)"),
        }
        // **全池 freepool 总览**：本池撕空但 freepool 空、live 归零 ⇒ 块很可能
        // 归还进了**别的池**（pool_of/feed 归属错判或跨核路径错位）——逐池打印
        // 各 power 链长，谁藏着巨链谁就是"错收"池。
        putln!("[block-oom] all-pool freepool census:");
        for (i, p) in self.blocks.iter().enumerate() {
            let Some(mut g) = p.inner.try_lock() else {
                putln!("[block-oom]   pool{i}: inner busy (skip)");
                continue;
            };
            let mut any = false;
            for (pw, head) in g.freepool.iter().enumerate() {
                let Some(mut node) = *head else { continue };
                let mut d = 0usize;
                loop {
                    d += 1;
                    if d > 1 << 16 {
                        break;
                    }
                    // SAFETY: freepool 节点恒为已释放块，首 8B 是 next。
                    let next = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
                    match next {
                        Some(n) => node = n,
                        None => break,
                    }
                }
                any = true;
                putln!(
                    "[block-oom]   pool{i} p{pw} ({} B): length {d}",
                    1usize << pw
                );
            }
            if !any {
                putln!("[block-oom]   pool{i}: all classes empty");
            }
        }
        // 全池 pump 在途块数：跨核释放滞留（泄漏）指标。
        for (i, p) in self.blocks.iter().enumerate() {
            for pw in MIN_POWER..=MAX_POWER {
                if let Some(g) = p.pump[pw].try_lock() {
                    if g.len != 0 {
                        putln!("[block-oom] pump[pool{i}][{pw}] in-flight {}", g.len);
                    }
                    drop(g);
                }
            }
        }
        // 业务分配调用点：portal 入口捕获（per-hart；`__rust_alloc` 内联后即
        // Vec/Box/Arc 等业务调用点）。block caller（=hybrid）与 block 内部分层
        // 无定位价值，真正的分配者在此 ra。
        let biz = crate::memory::allocator::portal::last_alloc_ra(machine::hart_id());
        putln!("[block-oom] portal caller (this hart) {biz:#x}");
    }
}

/// 核 → 节点 id：每核一池(UMA 全核同内存,节点 = 核的逻辑划分;多节点时
/// pools 数 = 核数,node_of 恒等直取)。
fn node_of(hart: usize) -> usize {
    hart
}

/// 读调用者返回地址（OOM 现场定位用；与 lock/depend::ra 同理，不依赖 audit
/// feature 的 re-export 门）。须在 allocate 入口捕获——ra 随后会被调用覆盖。
#[inline(always)]
fn caller_ra() -> usize {
    let ra: usize;
    // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
    unsafe { core::arch::asm!("mv {}, ra", out(reg) ra) };
    ra
}

/// 由请求 Layout 计算块 power（块 = 2^power 字节，须覆盖 size 且 ≤ 一页）。
fn block_power(layout: Layout) -> usize {
    let size = layout.size().max(1usize << MIN_POWER);
    let power = size.next_power_of_two().ilog2() as usize;
    power.clamp(MIN_POWER, MAX_POWER)
}

unsafe impl Allocator for BlockAllocator {
    /// 入口第一件事捕获调用者返回地址（OOM 现场用）——**必须先于任何函数调用**
    /// （caller_ra 内联后 asm 直读 ra；后续任何 jal 都会覆盖 ra）。inline(never)
    /// 保证读到的 ra 是分配调用者的返回地址（同 spin::lock 手法）。
    #[inline(never)]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let caller = caller_ra();
        let power = block_power(layout);
        if layout.align() > (1usize << power) {
            return Err(AllocError);
        }
        let me = node_of(machine::hart_id());
        let pool = &self.blocks[me];
        let Some(addr) = pool.pull(power) else {
            self.dump_oom(me, power, layout.size(), caller);
            return Err(AllocError);
        };
        // audit: 整块毒化（未初始化读现行）+ 活块入账（重复入账=块级双发现行；
        // KernelHeap 且 slack≥8 时 mark 内部写对齐 slack canary）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        {
            crate::memory::integrity::poison(addr, 1usize << power);
            crate::memory::integrity::LEDGER.mark(
                addr,
                layout.size(),
                crate::lock::ra(),
                crate::memory::integrity::OwnerKind::KernelHeap,
            );
        }
        // SAFETY: pull 返回的地址必非零（分配器保证）。
        Ok(NonNull::slice_from_raw_parts(
            unsafe { NonNull::new_unchecked(addr as *mut u8) },
            1usize << power,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let power = block_power(layout);
        let pa = ptr.addr().get();
        let Some(home) = self.pool_of(pa) else { return };
        // audit: 注销账目（无账=双 free/悬垂、canary=越界、尺寸=错幂现行）+ 本体毒化
        // 复写（头 8B 随后被 freelist 头插覆盖，其余保持毒化——UAF 读数变 0xCD）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        {
            crate::memory::integrity::LEDGER.unmark(pa, layout.size());
            crate::memory::integrity::poison(pa, 1usize << power);
        }
        let me = node_of(machine::hart_id());
        let pool = &self.blocks[home];
        if home == me {
            pool.push(ptr, power);
        } else {
            pool.feed(ptr, power);
        }
    }
}

// ── 每节点池：锁壳 + 区段 —— 等价 Frame 的 FrameAllocator 每节点一份 ──

/// 每个池从空闲区划走的堆区段，空间均分为两块：
/// 单元数组区（每块区页 512 B 位图）+ 块区（tear 撕页）。
/// 区段大小不设编译期常量——init 按「空闲区均分到每池」动态算出（见 init），
/// 大内存机器上自动放大；区段耗尽即内存耗尽。
pub(crate) struct BlockInner {
    inner: SpinLock<Pool>,                        // freepool + 页头 used 计数 + 区段游标
    pump: [SpinLock<Pump>; MAX_POWER + 1],        // 过境驿站：按 size class 分立；只收 feed，suck 抽空
    base: usize,                                  // 区段 [base, edge)，init 后不可变
    edge: usize,
}

impl BlockInner {
    fn new(base: usize, edge: usize, pool: Pool) -> BlockInner {
        BlockInner {
            inner: SpinLock::new(pool),
            pump: core::array::from_fn(|_| SpinLock::new(Pump::new())),
            base,
            edge,
        }
    }

    // ── 池内核心：拉 / 撕 / 推 ──

    /// 拉出一块：先 suck 归位过境块，再从 freelist 取；无则从区段撕页拆链。
    fn pull(&self, power: usize) -> Option<usize> {
        self.suck();
        let mut g = self.inner.lock();
        let inner = &mut *g;
        // debug: freepool 头必须在 DRAM 内（链表节点被覆写的特征——越界写/UAF）。
        if let Some(head) = inner.freepool[power] {
            #[cfg(debug_assertions)]
            {
                let m = machine::info();
                let a = head.as_ptr() as usize;
                if !m.free.range().contains(&a) {
                    panic!(
                        "block allocator: freelist head corrupted — power {power}, head {head:?} ({a:#x})"
                    );
                }
            }
            let next = unsafe { head.cast::<Option<NonNull<u8>>>().read() };
            inner.freepool[power] = next;
            unsafe { inner.increase_used(head, power) };
            inner.live[power] = inner.live[power].saturating_add(1);
            inner.allocs = inner.allocs.saturating_add(1);
            debug!("address {:?}, power {} allocated", head, power);
            return Some(head.as_ptr() as usize);
        }
        // 无现成块：撕下一页拆入链，首块即本次分配结果
        let first = self.tear(inner, power).ok()?;
        inner.live[power] = inner.live[power].saturating_add(1);
        inner.allocs = inner.allocs.saturating_add(1);
        Some(first.as_ptr() as usize)
    }

    /// 从区段 [base, edge) 撕下一页拆块入链，返回首块（即分配结果）。
    /// 调用方须已持本池 inner 锁（pull 内调用）。
    fn tear(&self, inner: &mut Pool, power: usize) -> Result<NonNull<u8>, AllocError> {
        let block_size = 1usize << power;
        if inner.cursor + PAGE_SIZE > self.edge {
            return Err(AllocError);
        }
        let page = inner.cursor;
        inner.cursor += PAGE_SIZE;

        // audit: Banker 取出本页（页来自自有区段，永驻不还；双取出现行）。
        #[cfg(all(debug_assertions, feature = "audit"))]
        crate::memory::integrity::BANKER.debit(page);

        if power < MAX_POWER {
            // 多块页：页头 8 字节存 used 计数，块从 offset 8 开始
            unsafe {
                *(page as *mut usize) = 1;
                let usable = page + 8;
                let block_nums = (PAGE_SIZE - 8) / block_size;
                link_blocks(usable, block_nums, block_size);
                let first = NonNull::new_unchecked(usable as *mut u8);
                // 其余块留在链上：首块的 next 指向块1…——首块本次被分配，其 next 字段
                // 将随 push 被覆盖（头插重写链头）而丢失，故先把链头写入 freepool
                // 再清空首块的 next，避免整链随首块丢失。
                inner.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
                first.cast::<Option<NonNull<u8>>>().write(None);
                Ok(first)
            }
        } else {
            // 整页单块：无页头
            unsafe {
                link_blocks(page, 1, block_size);
                let first = NonNull::new_unchecked(page as *mut u8);
                inner.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
                Ok(first)
            }
        }
    }

    /// 推回本池：写 freelist 链 + 递减本池页头计数（页永驻本池区段，不归还帧层）。
    fn push(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.inner.lock();
        let inner = &mut *g;

        // debug: double-free 检测——同一块已在 freepool 中再释放会破坏链表。
        #[cfg(debug_assertions)]
        {
            let mut cur = inner.freepool[power];
            let mut depth = 0usize;
            while let Some(node) = cur {
                if node == ptr {
                    self.dump_crash_site(inner, power, "push-double-free", ptr.as_ptr() as usize);
                    panic!("block allocator: double free of {:?} (power {power})", ptr);
                }
                depth += 1;
                if depth > 1 << 14 {
                    // debug: 链过长或成环——遍历失控前的护栏（环 = 某节点 next 被覆写）。
                    self.dump_crash_site(inner, power, "push-walk-loop", ptr.as_ptr() as usize);
                    panic!("block allocator: freepool[{power}] walk exceeded depth — cyclic list");
                }
                cur = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
            }
        }

        // 头插
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
        }
        inner.freepool[power] = Some(ptr);
        debug!("address {:?}, power {} deallocated", ptr, power);
        inner.live[power] = inner.live[power].saturating_sub(1);
        inner.frees = inner.frees.saturating_add(1);
        unsafe { inner.decrease_used(ptr, power) };
    }

    // ── 泵：喂 / 抽 ──

    /// 喂入本池 pump：块在外地被释放时投递至此（送它回家）。只入驿站，绝不碰 inner；
    /// 块首 8 字节挂链（同 freepool 手法），**零分配**——本路径常处 deallocate 持
    /// 门户锁的上下文，任何分配都会经 portal 递归重入（见 Pump 注释）。
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
                // 错，避免抽空死循环（同 push 的 freepool walk 护栏）。
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

    // ── 调试 ──

    /// 断言/push 异常现场 dump：游标/区段/该 power 全链/单元数组/操作序列。
    /// 注意：本函数**零分配**——任何 alloc 都会经 portal→block 递归/死锁，
    /// 且会污染现场。只用 putln! 直写 + 固定缓冲数组。
    #[cfg(debug_assertions)]
    fn dump_crash_site(&self, inner: &mut Pool, power: usize, ctx: &str, addr: usize) {
        putln!("[crash] {ctx}: failing block addr {addr:#x}");
        let torn_pages = (inner.cursor - self.base) / PAGE_SIZE;
        putln!(
            "[crash] {ctx}: pool base {:#x} edge {:#x} cursor {:#x} (torn {torn_pages} pages)",
            self.base,
            self.edge,
            inner.cursor
        );
        putln!("[crash] non-empty freepool classes:");
        for (p, h) in inner.freepool.iter().enumerate() {
            if h.is_some() {
                putln!("  class {p}");
            }
        }
        let mut walk = [0usize; 256];
        let mut n = 0usize;
        let mut cur = inner.freepool[power];
        while let Some(node) = cur {
            if n < walk.len() {
                walk[n] = node.as_ptr() as usize;
            }
            n += 1;
            cur = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
        }
        putln!(
            "[crash] freepool[{power}] walk ({} nodes, first 256 shown):",
            n
        );
        let shown = n.min(walk.len());
        (0..shown).for_each(|i| {
            let a = walk[i];
            putln!(
                "  [{}] {:#x} (page {:#x}, torn {}, offset {:#x})",
                i,
                a,
                a & !(crate::memory::PAGE_SIZE - 1),
                a < inner.cursor,
                a & (crate::memory::PAGE_SIZE - 1)
            );
        });
        // 游标后第一张未撕页原始内容（链头常指向未撕页——看那里有什么）
        if inner.cursor + PAGE_SIZE <= self.edge {
            let np = inner.cursor;
            let p = np as *const usize;
            putln!("[crash] first untorn page {np:#x} head words:");
            for i in 0..8 {
                putln!("  w{i} = {:#x}", unsafe { *p.add(i) });
            }
        }
        let blk = (addr & !(crate::memory::PAGE_SIZE - 1)) as *const usize;
        putln!("[crash] failing page head words:");
        for i in 0..8 {
            putln!("  w{i} = {:#x}", unsafe { *blk.add(i) });
        }
    }
}

// ── 池内状态：freepool + 游标 —— 等价 Frame 的 FrameInner ──

struct Pool {
    freepool: Vec<Option<NonNull<u8>>>,
    /// 区段游标：下一张待撕页的地址（仅 inner 锁内推进）。
    cursor: usize,
    /// per-power 活块净值（分配 +1 / 归还 -1；仅 inner 锁内改）。OOM 现场打印
    /// 「15.8 MB 被哪类块占」——泄漏的形状一眼可知（全 power 7 = 小而均匀的
    /// Arc/Vec 类泄漏；大 power = 块级泄漏）。
    live: Vec<usize>,
    /// 累计分配/归还次数（对数账：live 净值与 freepool 内容矛盾时，累计数揭示
    /// 是否有归还未发生）。
    allocs: u64,
    frees: u64,
}

impl Pool {
    fn new() -> Self {
        Self {
            freepool: Vec::new(),
            cursor: 0,
            live: Vec::new(),
            allocs: 0,
            frees: 0,
        }
    }

    fn init(&mut self, base: usize) -> Result<(), InitError> {
        self.freepool
            .try_reserve(MAX_POWER + 1)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freepool.resize_with(MAX_POWER + 1, || None);
        self.live.try_reserve(MAX_POWER + 1).map_err(|_| InitError::OutOfMemory)?;
        self.live.resize_with(MAX_POWER + 1, || 0);
        self.cursor = base;
        Ok(())
    }

    /// 标记某块在用数 +1（页头计数，仅 inner 锁内调用）。
    unsafe fn increase_used(&mut self, block: NonNull<u8>, power: usize) {
        unsafe {
            if power == MAX_POWER {
                return;
            }
            let base = block.as_ptr() as usize & !(PAGE_SIZE - 1);
            let used = &mut *(base as *mut usize);
            *used += 1;
        }
    }

    /// 标记某块在用数 -1（仅 inner 锁内调用）。计数归零的页留在本池区段——区段由
    /// tear 独占推进，绝不与 frame 区段重叠，页无需（也永不）归还帧层。
    ///
    /// **不得 purge**：曾在这里把 used==0 的整页从 freepool 摘链（purge_freelist），
    /// 而摘除的块永不重新入链——页内块全部归还即整页永久丢失（每页用空割一页，
    /// 池被撕空 → block-OOM 的根因）。块归还后都在 freepool 里正常循环，used 计数
    /// 仅是监控，页无需退役。
    unsafe fn decrease_used(&mut self, block: NonNull<u8>, power: usize) {
        unsafe {
            let base = block.as_ptr() as usize & !(PAGE_SIZE - 1);
            if power == MAX_POWER {
                return;
            }
            let used = &mut *(base as *mut usize);
            *used = used.saturating_sub(1);
            if *used > 0 {
                return;
            }
            // audit: 整页无活跃账目（used-counter 记账正确性检查；与 freepool
            // 内容无关——块都在链上正常周转）。
            #[cfg(all(debug_assertions, feature = "audit"))]
            crate::memory::integrity::page_clear(base);
        }
    }
}

// ── 自由助手 ──

/// 将 `block_nums` 个等大连续块串成单向链表。
unsafe fn link_blocks(base: usize, block_nums: usize, block_size: usize) {
    unsafe {
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
    }
}

// ── 静态实例 + 访问器 ──

static BLOCK_ALLOCATOR: OnceLock<BlockAllocator> = OnceLock::new();

fn heap() -> &'static BlockAllocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

/// 停机前把每个池的 pump 抽干（全部过境块归位后才能做帧基线断言）。
pub(crate) fn flush_all() {
    for pool in heap().blocks {
        pool.suck();
    }
}

// ── 适配层（hybrid/portal/tie 调用，接口零改动）──

/// audit: page 是否落在**任一**池区段内——供 frame 侧检测「frame 分配撞进
/// 块池区段页」（多池全查，单池退化为包含判定）；未初始化返回 false。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) fn pool_includes(pa: usize) -> bool {
    heap().pool_of(pa).is_some()
}

/// audit: 全部池已撕页总数（Banker audit 交叉核对用）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) fn torn_pages() -> usize {
    heap()
        .blocks
        .iter()
        .map(|p| {
            let this = &p;
            (this.inner.lock().cursor - this.base) / PAGE_SIZE
        })
        .sum()
}

pub fn allocator() -> &'static dyn Allocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

// ── 初始化 ──

/// 初始化块分配器（OnceLock 顶层单例装配；池/区段构建见 [`BlockAllocator::init`]）。
///
/// 必须在 `main` 早期调用恰好一次（经 `allocator::init`），bump 后端下执行；且须在
/// frame::init 之前——池区段划在 bump frontier 最前部，frame 从其后开始，两区不相交。
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
