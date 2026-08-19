// 块分配器 — 全局单一分池，segregated free list，单链表侵入式
//
// 将大块内存划分为 2^power 大小的 block，每块前 8 字节用 `Option<NonNull<u8>>`
// 存下一块的指针（利用 niche optimization: None = 0, Some = 指针值）。
// freepool[power] 指向该 size class 的空闲链表头部。
//
// 内存从 frame allocator 获取，按需懒分配新页。
// 每页单独追踪引用计数，全部 block 释放后整页归还。
// block 大小范围：2^3 .. 2^12（8 字节 .. 4096 字节 = PAGE_SIZE）。
// 最小对齐 8 字节，申请量不足 8 字节时自动向上取整。
//
// # 并发模型：全局单池 + SpinLock（不是 per-hart 池）
//
// 内核堆支撑着全部可迁移数据（Arc<Task>、Vec/Box/HashMap 缓冲、reaped 队列），
// 而调度器会跨 hart 偷取（steal）任务、任意 hart 在 clear() 里回收 Reaped 任务
// 并 drop 其 Arc ——分配与释放几乎必然发生在不同 hart。早期 per-hart 分池设计
// （each hart 自己的 freepool，无锁）在偷取引入后失效：跨池释放会
//  1. 把块挂进**错误的**池的空闲链；源池仍认为该块在用 → 重复分配同一块
//     （两个活持有者互相覆盖）；
//  2. `decrease_used` 在错误池的页上递减**垃圾位置**的页头计数，计数误归零后
//     把仍被堆使用的页整页归还 frame 池 → frame 分配器双重释放、
//     vtable/堆元数据被覆写 → 内核缺页 / 双重释放 panic。
// （实测症状：`&dyn Allocator::deallocate` vtable 分发缺页 stval=0x29；
//  `frame allocator: double free of index ... power 0`。）
// 故改为与 frame 分配器同构的**全局单池 + SpinLock**：锁即互斥，不再依赖
// 「分配与释放同 hart」的不变量（该不变量在任务迁移下无法成立）。
//
// 锁序：block → frame（refill 取页 / decrease_used 还页会调 frame 分配器），
// 从不反向——frame 分配器的分配/释放路径不做任何全局堆分配（其元数据 Vec
// 在 init 期经 bump 预分配且此后只索引不增长）。
use core::ptr::NonNull;

use core::alloc::{AllocError, Allocator, Layout};

use alloc::vec::Vec;
use erra::ResultExt;
use log::debug;

use crate::memory::PAGE_SIZE;
use crate::{
    lock::SpinLock,
    memory::allocator::{InitError, InitResult, frame::allocator as frame_allocator},
};

const MIN_POWER: usize = 3;
const MAX_POWER: usize = PAGE_SIZE.ilog2() as usize;

pub(crate) struct BlockAllocator {
    inner: SpinLock<Option<BlockInner>>,
}

// SAFETY: 全部可变状态经 SpinLock 访问（同 frame 分配器），同时刻只有一把
// guard（互斥 + 中断关闭），可安全跨 hart 共享。
unsafe impl Sync for BlockAllocator {}

impl BlockAllocator {
    const fn new() -> Self {
        Self {
            inner: SpinLock::new(None),
        }
    }

    pub fn init(&self) -> Result<(), InitError> {
        let mut guard = self.inner.lock();
        // SAFETY: 单 hart（boot 早期）下调用，无并发。
        let mut inner = BlockInner::new();
        inner.init()?;
        // debug: unitmap 基准 = frame 区域基址（与 frame::init 同一算式；
        // frame::init 后置写入相同值）。
        #[cfg(debug_assertions)]
        {
            let base = crate::memory::allocator::bump::frontier().next_multiple_of(PAGE_SIZE);
            unitmap::set_base(base);
        }
        guard.replace(inner);
        Ok(())
    }
}

unsafe impl Allocator for BlockAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let power = block_power(layout);
        let block_size = 1usize << power;

        if layout.align() > block_size {
            return Err(AllocError);
        }

        // SAFETY: SpinLockGuard 关中断 + 互斥，防止同核中断重入与跨核并发。
        let mut guard = self.inner.lock();
        let inner = guard.as_mut().ok_or(AllocError)?;

        // 从 freelist 头部弹出
        if let Some(head) = inner.freepool[power] {
            // debug: freepool 头必须是 DRAM 内的合法地址——否则 free list
            // 已被覆写（越界写/use-after-free 特征），读它必崩。提前报出
            // size class 与调用点，而非事后在错误地址上 page fault。
            #[cfg(debug_assertions)]
            {
                use crate::machine;

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
            // debug: 块级在位标记（抓堆内块双发）
            #[cfg(debug_assertions)]
            unitmap::mark(head.as_ptr() as usize, block_size);

            debug!("address {:?}, power {} allocated", head, power);

            return Ok(NonNull::slice_from_raw_parts(head, block_size));
        }

        unsafe { inner.refill(power) }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let power = block_power(layout);

        // SAFETY: SpinLockGuard 关中断 + 互斥，防止同核中断重入与跨核并发。
        let mut guard = self.inner.lock();
        let Some(inner) = guard.as_mut() else {
            return;
        };

        // debug: double-free 检测——同一 block 已在 freepool 中再释放
        // 会破坏链表（两次分配同一块 → 重叠写坏）。遍历当前 size class。
        #[cfg(debug_assertions)]
        {
            let mut cur = inner.freepool[power];
            while let Some(node) = cur {
                if node == ptr {
                    panic!("block allocator: double free of {:?} (power {power})", ptr);
                }
                // SAFETY: freepool 节点恒为已释放块，头 8 字节是 next 指针。
                cur = node.cast::<Option<NonNull<u8>>>().read();
            }
        }

        // 头插：将 freed block 写入 freelist 头部
        // debug: 释放前先校验块确在途（错幂释放/脏指针的面具在此揭开）
        #[cfg(debug_assertions)]
        unitmap::unmark(ptr.as_ptr() as usize, 1usize << power);
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
        }
        inner.freepool[power] = Some(ptr);

        debug!("address {:?}, power {} deallocated", ptr, power);

        // 该 pool 在用数 -1，归零时整页归还
        unsafe { inner.decrease_used(ptr, power) };
    }
}

struct BlockInner {
    freepool: Vec<Option<NonNull<u8>>>,
}

impl BlockInner {
    fn new() -> Self {
        Self {
            freepool: Vec::new(),
        }
    }

    fn init(&mut self) -> Result<(), InitError> {
        self.freepool
            .try_reserve(MAX_POWER + 1)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freepool.resize_with(MAX_POWER + 1, || None);
        Ok(())
    }

    /// 标记某 block 在用数 +1。
    ///
    /// # Safety
    ///
    /// `block` 必须来自本分配器 refill 的页。
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

    /// 标记某 block 在用数 -1。归零时整页归还。
    ///
    /// # Safety
    ///
    /// `block` 必须来自本分配器 refill 的页。
    unsafe fn decrease_used(&mut self, block: NonNull<u8>, power: usize) {
        unsafe {
            let base = block.as_ptr() as usize & !(PAGE_SIZE - 1);
            if power == MAX_POWER {
                self.freepool[power] = purge_freelist(self.freepool[power], base);
                #[cfg(debug_assertions)]
                {
                    HEAP_LIVE_PAGES.fetch_sub(1, Ordering::Relaxed);
                    // debug: 整页归还——页必须确实由堆持有（防把非堆页误还）
                    crate::memory::allocator::pageown::release(base);
                    unitmap::assert_page_clear(base);
                }
                frame_allocator().deallocate(
                    NonNull::new_unchecked(base as *mut u8),
                    Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap(),
                );
                return;
            }

            let used = &mut *(base as *mut usize);
            *used = used.saturating_sub(1);
            if *used > 0 {
                return;
            }

            self.freepool[power] = purge_freelist(self.freepool[power], base);
            #[cfg(debug_assertions)]
            {
                HEAP_LIVE_PAGES.fetch_sub(1, Ordering::Relaxed);
                // debug: 整页归还——页必须确实由堆持有（防把非堆页误还）
                crate::memory::allocator::pageown::release(base);
                unitmap::assert_page_clear(base);
            }
            frame_allocator().deallocate(
                NonNull::new_unchecked(base as *mut u8),
                Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap(),
            );
        }
    }

    unsafe fn refill(&mut self, power: usize) -> Result<NonNull<[u8]>, AllocError> {
        unsafe {
            let block_size = 1usize << power;

            let page = frame_allocator()
                .allocate(Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap())
                .map_err(|_| AllocError)?;
            #[cfg(debug_assertions)]
            HEAP_LIVE_PAGES.fetch_add(1, Ordering::Relaxed);

            let base = page.cast::<u8>().as_ptr() as usize;
            // debug: 登记堆页所有权（页必须来自 frame 池且此前未被堆持有）
            #[cfg(debug_assertions)]
            crate::memory::allocator::pageown::hold(base);

            if power < MAX_POWER {
                // 多 block 页：页头 8 字节存 used 计数，block 从 offset 8 开始
                *(base as *mut usize) = 1;
                let usable = base + 8;
                let block_nums = (PAGE_SIZE - 8) / block_size;
                link_blocks(usable, block_nums, block_size);

                let first = NonNull::new_unchecked(usable as *mut u8);
                self.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
                // debug: 首块随 refill 分配（在途标记）
                #[cfg(debug_assertions)]
                unitmap::mark(first.as_ptr() as usize, block_size);
                Ok(NonNull::slice_from_raw_parts(first, block_size))
            } else {
                // 整页单 block：无页头
                link_blocks(base, 1, block_size);

                let first = NonNull::new_unchecked(base as *mut u8);
                self.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
                // debug: 首块随 refill 分配（在途标记）
                #[cfg(debug_assertions)]
                unitmap::mark(first.as_ptr() as usize, block_size);
                Ok(NonNull::slice_from_raw_parts(first, block_size))
            }
        }
    }
}

fn block_power(layout: Layout) -> usize {
    let size = layout.size().max(1usize << MIN_POWER);
    let power = size.next_power_of_two().ilog2() as usize;
    power.clamp(MIN_POWER, MAX_POWER)
}

/// 将 `block_nums` 个等大连续 block 串成单向链表。
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

/// 遍历 freepool，移除属于指定 pool（页）的所有 block 条目。
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

        // 尾 block 的 next 置 None
        if let Some(p) = last {
            p.cast::<Option<NonNull<u8>>>().write(None);
        }

        new_head
    }
}

/// 全局单例：唯一的内核堆（与 frame 分配器同构：SpinLock 保护，跨 hart 共享）。
static BLOCK_ALLOCATOR: BlockAllocator = BlockAllocator::new();

/// 内核堆当前从 FRAME_ALLOCATOR 持有的页数（debug 统计用；release 下恒 0）。
///
/// `refill` 每取一页 +1，整页归还（`decrease_used` 两条路径）各 -1。关机时仍
/// 持有的页是堆的常驻支撑内存（任务帧之外），`check_baseline` 需扣除——否则
/// 把良性的堆页误报为任务帧泄漏。
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(debug_assertions)]
static HEAP_LIVE_PAGES: AtomicUsize = AtomicUsize::new(0);

/// 堆持有的物理页数（debug 统计用；仅 debug 构建存在）。
#[cfg(debug_assertions)]
pub(crate) fn live_pages() -> usize {
    HEAP_LIVE_PAGES.load(Ordering::Relaxed)
}

// ── 块级在位位图（debug）：任何块在被分配时必须是「无主」的 ──
//
// 页级 pageown 位图只能抓「堆页泄漏进 frame 池」；本位图按 8 字节单元追踪
// 「块当前有没有活跃持有者」——抓到**堆内部**的块级双发（同一块发给两个持有者
//  → Arc<Task> 头互写、strong 幻值）与错幂释放（如 4096 布局释放在 128B 块页
//  → 整页被误还/整页单元断言失败）。诊断成本：2 MB .bss（仅 debug 构建）。
//
// 仅在 BLOCK 锁内访问（分配/释放路径已持锁），AtomicU8 只作静态内存容器。
#[cfg(debug_assertions)]
mod unitmap {
    use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
    use crate::memory::PAGE_SIZE;

    pub(crate) const UNITS_PER_PAGE: usize = PAGE_SIZE / 8; // 512 单元/页
    pub(crate) const PAGES: usize = 32768; // 128 MiB / 4 KiB
    /// 单元在途位（1 = 有活跃持有者；0 = 空闲/在 freepool）。
    pub(crate) static FLAG: [AtomicU8; UNITS_PER_PAGE * PAGES] =
        [const { AtomicU8::new(0) }; UNITS_PER_PAGE * PAGES];
    static FRAME_BASE: AtomicUsize = AtomicUsize::new(0);

    pub(crate) fn set_base(base: usize) {
        FRAME_BASE.store(base, Ordering::Relaxed);
    }

    fn base() -> usize {
        FRAME_BASE.load(Ordering::Relaxed)
    }

    /// 全局单元号：相对 frame 基址的 (页内偏移 8B 单元)。
    fn unit(addr: usize) -> usize {
        let b = base();
        let page = (addr - b) / PAGE_SIZE;
        assert!(
            page < PAGES,
            "unitmap: addr {addr:#x} outside range (base {b:#x})"
        );
        page * UNITS_PER_PAGE + (addr % PAGE_SIZE) / 8
    }

    /// 断言 addr..addr+size 所有单元均无主（再标记为在途）。
    pub(crate) fn mark(addr: usize, size: usize) {
        let n = size / 8;
        let u0 = unit(addr);
        for i in 0..n {
            let f = FLAG[u0 + i].load(Ordering::Relaxed);
            assert_eq!(
                f, 0,
                "block alloc: unit {} (addr {:#x}) ALREADY IN USE — 块级双发！(base {:#x})",
                u0 + i, addr + i * 8, base()
            );
            FLAG[u0 + i].store(1, Ordering::Relaxed);
        }
    }

    /// 断言 addr..addr+size 所有单元均在途（再清空）。
    pub(crate) fn unmark(addr: usize, size: usize) {
        let n = size / 8;
        let u0 = unit(addr);
        for i in 0..n {
            let f = FLAG[u0 + i].load(Ordering::Relaxed);
            assert_eq!(
                f, 1,
                "block dealloc: unit {} (addr {:#x}) NOT IN USE — 释放未分配单元（错幂释放/脏指针）",
                u0 + i, addr + i * 8
            );
            FLAG[u0 + i].store(0, Ordering::Relaxed);
        }
    }

    /// 断言整页（header 之外）无活跃单元——整页归还前的完整性检查。
    pub(crate) fn assert_page_clear(pa: usize) {
        let u0 = unit(pa);
        debug_assert_eq!(FLAG[u0].load(Ordering::Relaxed), 0, "unitmap: page header unit busy");
        for i in 1..UNITS_PER_PAGE {
            let f = FLAG[u0 + i].load(Ordering::Relaxed);
            assert_eq!(
                f, 0,
                "block: page {pa:#x} returned with live unit {} — 计数记账错误（used-counter 提前归零）",
                i
            );
        }
    }

    /// 初始化基准（block::init 时由 frame 侧 base 写入；refill 前必有值）。
    pub(crate) fn base_set() -> bool {
        base() != 0
    }
}

pub fn allocator() -> &'static dyn Allocator {
    &BLOCK_ALLOCATOR
}

/// 初始化块分配器。
///
/// 必须在 `main` 早期调用恰好一次（经 `allocator::init`），在任何堆分配之前。
/// 此时门户分配器仍在 bump 后端，本模块自身的 Vec 元数据经 bump 分配——
/// 不会重入本锁。
///
/// # Errors
///
/// 元数据分配失败 → [`InitError::OutOfMemory`]。
pub fn init() -> InitResult<()> {
    BLOCK_ALLOCATOR
        .init()
        .annotate("initializing block allocator")
}