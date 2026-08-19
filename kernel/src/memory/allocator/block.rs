// 块分配器 — per-node 池 + 泵（pump）过境路由（segregated free list，单链表侵入式）
//
// 块堆按节点分池：每个节点一个 Pool，池自带物理区段 [base, edge)；节点内多核共享
// 一把 inner 锁；块从池区段上撕页（tear）拆链——页来自自有区段，**不经过帧分配器**
// （帧层保持全局，只管栈/trap/页表页）。跨节点释放经泵路由：块在异地释放时 feed 进
// 其归属池的 pump 驿站，属主核下次 pull 前 suck 抽回归位——正确性不依赖任何调度策略
// （任务可自由迁移，内存子系统免疫调度行为）。
//
// 命名：pool(池) pump(泵) pull/push(池内拉/推) feed/suck(泵口喂/抽) tear(撕页)——全部
// 4 字母动词成族；base/edge 区段两端成对；pool_of/node_of 是定位原语。
//
// 不变·硬（贴结构）：
//   - 块只进归属池的 freelist：feed 只入 pump，suck 是唯一转 push 的路径；
//   - 页头 used 计数只在池 inner 锁内写（feed 拿不到 inner）；
//   - 拓扑（区段表）建成后只读；锁序 = pull/suck 先 pump 后 inner（摘空再归位），
//     feed 仅 pump，push/tear 仅 inner——无环。
use core::ptr::NonNull;

use core::alloc::{AllocError, Allocator, Layout};

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use erra::ResultExt;
use log::debug;

use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::{
    lock::{OnceLock, SpinLock},
    memory::allocator::{InitError, InitResult, bump},
};

const MIN_POWER: usize = 3;
const MAX_POWER: usize = PAGE_SIZE.ilog2() as usize;

/// 每个池从空闲区划走的堆区段总大小（16 MiB），内部分两块：
/// 单元数组区（ARRAY_SIZE，覆盖块区每页的块级在位位图）+ 块区（tear 撕页）。
/// spawner 8 核场景峰值实测数百页，块区 14 MiB = 3584 页富余；区段耗尽即内存耗尽。
const POOL_SIZE: usize = 16 * 1024 * 1024;
/// 单元数组区大小：块区页数 × 512 B（3584 × 512 ≈ 1.75 MiB，取 2 MiB 富余）。
const ARRAY_SIZE: usize = 2 * 1024 * 1024;

pub(crate) struct Pool {
    inner: SpinLock<Option<BlockInner>>, // freepool + 页头 used 计数 + 区段游标
    pump: SpinLock<VecDeque<Remote>>,     // 过境驿站：只收 feed，suck 抽空
    base: usize,                          // 区段 [base, edge)，init 后不可变
    edge: usize,
}

/// 过境块：喂入/吸出之间携带的最小信息（指针 + size class）。
struct Remote {
    ptr: NonNull<u8>,
    power: usize,
}

impl Pool {
    fn new(base: usize, edge: usize) -> Pool {
        Pool {
            inner: SpinLock::new(None),
            pump: SpinLock::new(VecDeque::new()),
            base,
            edge,
        }
    }

    fn init(&self) -> Result<(), InitError> {
        let mut inner = BlockInner::new();
        inner.init(self.base)?;
        let mut g = self.inner.lock();
        g.replace(inner);
        Ok(())
    }

    /// 拉出一块：先 suck 归位过境块，再从 freelist 取；无则从区段撕页拆链。
    fn pull(&self, power: usize) -> Option<usize> {
        self.suck();
        let mut g = self.inner.lock();
        let inner = g.as_mut()?;
        // debug: freepool 头必须在 DRAM 内（链表节点被覆写的特征——越界写/UAF）。
        if let Some(head) = inner.freepool[power] {
            #[cfg(debug_assertions)]
            {
                let m = machine::get();
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
            // debug: 弹出块必须无主（块级双发检测），再标在途。
            #[cfg(debug_assertions)]
            unitmap::mark(head.as_ptr() as usize, 1usize << power);
            debug!("address {:?}, power {} allocated", head, power);
            return Some(head.as_ptr() as usize);
        }
        // 无现成块：撕下一页拆入链，首块即本次分配结果
        self.tear(inner, power)
            .ok()
            .map(|first| first.as_ptr() as usize)
    }

    /// 推回本池：写 freelist 链 + 递减本池页头计数（页永驻本池区段，不归还帧层）。
    fn push(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.inner.lock();
        let Some(inner) = g.as_mut() else { return };

        // debug: double-free 检测——同一块已在 freepool 中再释放会破坏链表。
        #[cfg(debug_assertions)]
        {
            let mut cur = inner.freepool[power];
            while let Some(node) = cur {
                if node == ptr {
                    panic!("block allocator: double free of {:?} (power {power})", ptr);
                }
                cur = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
            }
        }

        // debug: 释放前先校验块确在途（错幂释放/脏指针的面具在此揭开）。
        #[cfg(debug_assertions)]
        unitmap::unmark(ptr.as_ptr() as usize, 1usize << power);
        // 头插
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
        }
        inner.freepool[power] = Some(ptr);
        debug!("address {:?}, power {} deallocated", ptr, power);
        unsafe { inner.decrease_used(ptr, power) };
    }

    /// 喂入本池 pump：块在外地被释放时投递至此（送它回家）。只入驿站，绝不碰 inner。
    fn feed(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pump.lock();
        g.push_back(Remote { ptr, power });
    }

    /// 抽干本池 pump：取空队列，逐块归位（锁序 pump→放→inner；幂等）。
    fn suck(&self) {
        // 交换队列：锁内只做 O(1) 摘空，绝不持 pump 求 inner
        let drained = {
            let mut g = self.pump.lock();
            core::mem::take(&mut *g)
        };
        for Remote { ptr, power } in drained {
            self.push(ptr, power);
        }
    }

    /// 从区段 [base, edge) 撕下一页拆块入链，返回首块（即分配结果）。
    /// 调用方须已持本池 inner 锁（pull 内调用）。
    fn tear(&self, inner: &mut BlockInner, power: usize) -> Result<NonNull<u8>, AllocError> {
        let block_size = 1usize << power;
        if inner.cursor + PAGE_SIZE > self.edge {
            return Err(AllocError);
        }
        let page = inner.cursor;
        inner.cursor += PAGE_SIZE;

        // debug: 登记堆页所有权（页来自自有区段，永驻不还）。单元位图已由 init
        // 静态映射（数组区），无需逐页分配。
        #[cfg(debug_assertions)]
        crate::memory::allocator::pageown::hold(page);

        if power < MAX_POWER {
            // 多块页：页头 8 字节存 used 计数，块从 offset 8 开始
            unsafe {
                *(page as *mut usize) = 1;
                let usable = page + 8;
                let block_nums = (PAGE_SIZE - 8) / block_size;
                link_blocks(usable, block_nums, block_size);
                let first = NonNull::new_unchecked(usable as *mut u8);
                // debug: 首块随撕页分配（在途标记）
                #[cfg(debug_assertions)]
                unitmap::mark(first.as_ptr() as usize, block_size);
                Ok(first)
            }
        } else {
            // 整页单块：无页头
            unsafe {
                link_blocks(page, 1, block_size);
                let first = NonNull::new_unchecked(page as *mut u8);
                inner.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
                // debug: 首块随撕页分配（在途标记）
                #[cfg(debug_assertions)]
                unitmap::mark(first.as_ptr() as usize, block_size);
                Ok(first)
            }
        }
    }
}

struct BlockInner {
    freepool: Vec<Option<NonNull<u8>>>,
    /// 区段游标：下一张待撕页的地址（仅 inner 锁内推进）。
    cursor: usize,
}

impl BlockInner {
    fn new() -> Self {
        Self {
            freepool: Vec::new(),
            cursor: 0,
        }
    }

    fn init(&mut self, base: usize) -> Result<(), InitError> {
        self.freepool
            .try_reserve(MAX_POWER + 1)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freepool.resize_with(MAX_POWER + 1, || None);
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
    unsafe fn decrease_used(&mut self, block: NonNull<u8>, power: usize) {
        unsafe {
            let base = block.as_ptr() as usize & !(PAGE_SIZE - 1);
            if power == MAX_POWER {
                self.freepool[power] = purge_freelist(self.freepool[power], base);
                // debug: 整页清链后须无活跃单元（used-counter 记账正确性检查）。
                #[cfg(debug_assertions)]
                unitmap::assert_page_clear(base);
                return;
            }
            let used = &mut *(base as *mut usize);
            *used = used.saturating_sub(1);
            if *used > 0 {
                return;
            }
            self.freepool[power] = purge_freelist(self.freepool[power], base);
            // debug: 同上——整页清链后须无活跃单元（页永驻，仅校验，不归还）。
            #[cfg(debug_assertions)]
            unitmap::assert_page_clear(base);
        }
    }
}

fn block_power(layout: Layout) -> usize {
    let size = layout.size().max(1usize << MIN_POWER);
    let power = size.next_power_of_two().ilog2() as usize;
    power.clamp(MIN_POWER, MAX_POWER)
}

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

/// 遍历 freepool，移除属于指定页的所有块条目。
unsafe fn purge_freelist(head: Option<NonNull<u8>>, pool_base: usize) -> Option<NonNull<u8>> {
    unsafe {
        let pool_end = pool_base + PAGE_SIZE;
        let mut new_head = None;
        let mut last: Option<NonNull<u8>> = None;
        let mut this = head;

        while let Some(node) = this {
            let addr = node.as_ptr() as usize;
            let next: Option<NonNull<u8>> = node.cast::<Option<NonNull<u8>>>().read();

            if !(addr >= pool_base && addr < pool_end) {
                if new_head.is_none() {
                    new_head = Some(node);
                }
                if let Some(p) = last {
                    p.cast::<Option<NonNull<u8>>>().write(Some(node));
                }
                last = Some(node);
            }

            this = next;
        }

        if let Some(p) = last {
            p.cast::<Option<NonNull<u8>>>().write(None);
        }

        new_head
    }
}

// ── 全局核心：池集合 + 区段表 ──

struct BlockHeap {
    pools: &'static [Pool],
    segments: &'static [Segment],
}

/// 区段：一段物理内存 → 池 id（按 base 有序，二分查询；init 后只读）。
struct Segment {
    base: usize,
    edge: usize,
    pool: usize,
}

static BLOCK_HEAP: OnceLock<&'static BlockHeap> = OnceLock::new();

fn heap() -> &'static BlockHeap {
    BLOCK_HEAP.get().expect("block heap not initialized")
}

/// 页地址 → 池 id（区段覆盖判定；非常规堆内存返回 None）。
fn pool_of(pa: usize) -> Option<usize> {
    let segs = heap().segments;
    // 二分：找最后一个 base <= pa 的段
    let idx = segs.partition_point(|s| s.base <= pa);
    if idx == 0 {
        return None;
    }
    let s = &segs[idx - 1];
    (pa < s.edge).then_some(s.pool)
}

/// 核 → 节点 id（当前机器全核同节点，单段内存；多节点时按拓扑填充）。
fn node_of(_hart: usize) -> usize {
    0
}

/// 停机前把每个池的 pump 抽干（全部过境块归位后才能做帧基线断言）。
pub(crate) fn flush_all() {
    for pool in heap().pools {
        pool.suck();
    }
}

// ── 适配层（hybrid/portal/tie 调用，接口零改动）──

pub struct BlockAllocator;

unsafe impl Allocator for BlockAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let power = block_power(layout);
        if layout.align() > (1usize << power) {
            return Err(AllocError);
        }
        let me = node_of(machine::hart_id());
        let pool = &heap().pools[me];
        let addr = pool.pull(power).ok_or(AllocError)?;
        // SAFETY: pull 返回的地址必非零（分配器保证）。
        Ok(NonNull::slice_from_raw_parts(
            unsafe { NonNull::new_unchecked(addr as *mut u8) },
            1usize << power,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            let power = block_power(layout);
            let pa = ptr.addr().get();
            let Some(home) = pool_of(pa) else { return };
            let me = node_of(machine::hart_id());
            let pool = &heap().pools[home];
            if home == me {
                pool.push(ptr, power);
            } else {
                pool.feed(ptr, power);
            }
        }
    }
}

static BLOCK_ALLOCATOR: BlockAllocator = BlockAllocator;

/// 块池堆借自帧层的页数：恒 0——池区段自给自足，从不向帧层借用。
/// 保留该接口仅为 frame::check_baseline 的堆扣除公式兼容（扣除项恒零即退化正确）。
pub fn live_pages() -> usize {
    0
}

pub fn allocator() -> &'static dyn Allocator {
    &BLOCK_ALLOCATOR
}

// ── 块级在位位图（debug）：任何块在被分配时必须是「无主」的 ──
//
// 页级 pageown 位图只能抓「堆页泄漏进 frame 池」；本位图按 8 字节单元追踪
// 「块当前有没有活跃持有者」——抓到**堆内部**的块级双发（同一块发给两个持有者
//  → Arc<Task> 头互写、strong 幻值）与错幂释放（如 4096 布局释放在 128B 块页）。
//
// 存储：**池区段内的固定数组区**（ARRAY_SIZE，每块区页 512 B 单元数组）——静态
// 映射、零运行时分配：flags(pa) = 数组区 + 页偏移 × 512，O(1) 无锁直查。不向
// frame 池借页（否则数组帧随页永驻而泄漏，boot selftest 帧基线即暴露）。仅在池
// inner 锁内访问（分配/释放路径已持锁），无需原子指令。
#[cfg(debug_assertions)]
mod unitmap {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use crate::memory::PAGE_SIZE;

    pub(crate) const UNITS_PER_PAGE: usize = PAGE_SIZE / 8; // 512 单元/页
    const UNIT_BYTES: usize = 512; // 每页单元数组字节数

    /// 块区基址（页索引基准）与数组区基址；init 写入一次，此后只读。
    static POOL_BASE: AtomicUsize = AtomicUsize::new(0);
    static ARRAY_BASE: AtomicUsize = AtomicUsize::new(0);
    /// 数组区覆盖的块区页数（上限校验用）。
    static ARRAY_PAGES: AtomicUsize = AtomicUsize::new(0);

    /// 记录基准（block::init 调用恰好一次，单 hart boot 期）。
    /// 数组区与块区相邻：单元数组静态映射，无需逐页分配/归还。
    pub(crate) fn set(pool_base: usize, array_base: usize, array_pages: usize) {
        assert_eq!(
            array_base % PAGE_SIZE,
            0,
            "unitmap: array base must be page-aligned"
        );
        POOL_BASE.store(pool_base, Ordering::Relaxed);
        ARRAY_BASE.store(array_base, Ordering::Relaxed);
        ARRAY_PAGES.store(array_pages, Ordering::Relaxed);
    }

    /// 某页的单元数组指针（页须在块区内；块页必然在区内）。
    fn flags(pa: usize) -> *mut u8 {
        let idx = (pa - POOL_BASE.load(Ordering::Relaxed)) / PAGE_SIZE;
        let pages = ARRAY_PAGES.load(Ordering::Relaxed);
        assert!(
            idx < pages,
            "unitmap: page {pa:#x} outside block region (idx {idx}, pages {pages})"
        );
        (ARRAY_BASE.load(Ordering::Relaxed) + idx * UNIT_BYTES) as *mut u8
    }

    /// 断言 addr..addr+size 所有单元均无主（再标记为在途）。
    pub(crate) fn mark(addr: usize, size: usize) {
        let n = size / 8;
        let u0 = (addr % PAGE_SIZE) / 8;
        let flags = flags(addr);
        for i in 0..n {
            // SAFETY: u0+n ≤ 512（块不跨页，size ≤ PAGE_SIZE）；页在块区内。
            let f = unsafe { *flags.add(u0 + i) };
            assert_eq!(
                f, 0,
                "block alloc: unit {} (addr {:#x}) ALREADY IN USE — 块级双发！",
                u0 + i, addr + i * 8
            );
            unsafe { *flags.add(u0 + i) = 1 };
        }
    }

    /// 断言 addr..addr+size 所有单元均在途（再清空）。
    pub(crate) fn unmark(addr: usize, size: usize) {
        let n = size / 8;
        let u0 = (addr % PAGE_SIZE) / 8;
        let flags = flags(addr);
        for i in 0..n {
            // SAFETY: 同 mark。
            let f = unsafe { *flags.add(u0 + i) };
            assert_eq!(
                f, 1,
                "block dealloc: unit {} (addr {:#x}) NOT IN USE — 释放未分配单元（错幂释放/脏指针）",
                u0 + i, addr + i * 8
            );
            unsafe { *flags.add(u0 + i) = 0 };
        }
    }

    /// 断言整页无活跃单元——页级记账完整性检查（页永驻池区段，仅校验不归还）。
    pub(crate) fn assert_page_clear(pa: usize) {
        let flags = flags(pa);
        for i in 0..UNITS_PER_PAGE {
            // SAFETY: 页在块区内，i < 512。
            let f = unsafe { *flags.add(i) };
            assert_eq!(
                f, 0,
                "block: page {pa:#x} returned with live unit {} — 计数记账错误（used-counter 提前归零）",
                i
            );
        }
    }
}

/// 初始化块分配器：从空闲区划走每个池的区段（数组区 + 块区），建池集合 + 区段表。
///
/// 必须在 `main` 早期调用恰好一次（经 `allocator::init`），bump 后端下执行——池
/// 元数据经 bump 分配，不会重入本锁。必须在 bump 的所有元数据分配之后、frame 的
/// base 计算之前：池区段划在 bump frontier 最前部，frame 从其后开始，两区不相交。
///
/// # Errors
///
/// 元数据分配失败 / 区段划走失败 → [`InitError::OutOfMemory`]。
pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        // 区段：从 bump 前端划走 POOL_SIZE（单节点；多节点按拓扑逐段规划）
        let layout = Layout::from_size_align(POOL_SIZE, PAGE_SIZE).unwrap();
        let region = bump::allocator()
            .allocate(layout)
            .map_err(|_| InitError::OutOfMemory)?;
        let region_base = region.as_ptr() as *const u8 as usize;
        // 布局：数组区在区段头部，块区在其后；array 永不参与块分配。
        let array_base = region_base;
        let base = region_base + ARRAY_SIZE;
        let edge = region_base + POOL_SIZE;
        let block_pages = (POOL_SIZE - ARRAY_SIZE) / PAGE_SIZE;

        // debug: 页级位图按空闲区索引（frame 侧检查用）；块级位图按块区静态映射。
        #[cfg(debug_assertions)]
        {
            let m = machine::get();
            let fbase = m.free.base;
            let fpages = m.free.size / PAGE_SIZE;
            crate::memory::allocator::pageown::set_base(fbase, fpages);
            unitmap::set(base, array_base, block_pages);
        }

        let pool = Pool::new(base, edge);
        pool.init()?;
        let pools = Box::leak(vec![pool].into_boxed_slice());
        let segments = Box::leak(vec![Segment { base, edge, pool: 0 }].into_boxed_slice());
        let heap = Box::leak(Box::new(BlockHeap { pools, segments }));
        if BLOCK_HEAP.set(heap).is_ok() {
            Ok(())
        } else {
            Err(InitError::AlreadyInitialized)
        }
    })()
    .annotate("initializing block allocator")
}