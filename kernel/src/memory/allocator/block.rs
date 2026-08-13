// 块分配器 — segregated free list，单链表侵入式
//
// 将大块内存划分为 2^power 大小的 block，每块前 8 字节用 `Option<NonNull<u8>>`
// 存下一块的指针（利用 niche optimization: None = 0, Some = 指针值）。
// freepool[power] 指向该 size class 的空闲链表头部。
//
// 内存从 frame allocator（buddy）获取，按需懒分配新页。
// 每页单独追踪引用计数，全部 block 释放后整页归还。
// block 大小范围：2^3 .. 2^12（8 字节 .. 4096 字节 = PAGE_SIZE）。
// 最小对齐 8 字节，申请量不足 8 字节时自动向上取整。

#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{cell::UnsafeCell, ptr::NonNull};

use core::alloc::{AllocError, Allocator, Layout};

use alloc::{boxed::Box, vec::Vec};

use crate::{
    lock::{OnceLock, TrapGuard},
    memory::PAGE_SIZE,
    memory::allocator::frame::allocator as frame_allocator,
};

const MIN_POWER: usize = 3;
const MAX_POWER: usize = PAGE_SIZE.ilog2() as usize;

pub(crate) struct BlockAllocator {
    inner: UnsafeCell<Option<BlockInner>>,
}

// SAFETY: per-hart（每个核只访问自己的 slot），TrapGuard 防止同核中断重入。
unsafe impl Sync for BlockAllocator {}

impl BlockAllocator {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(None),
        }
    }

    pub fn init(&self) {
        // SAFETY: 单 hart 下调用，无并发。
        let cell = unsafe { &mut *self.inner.get() };
        cell.replace({
            let mut inner = BlockInner::new();
            inner.init();
            inner
        });
    }
}

unsafe impl Allocator for BlockAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let power = block_power(layout);
        let block_size = 1usize << power;

        if layout.align() > block_size {
            return Err(AllocError);
        }

        // SAFETY: TrapGuard 关中断防止同核重入；per-hart 保证不被其他核访问。
        unsafe {
            let _trap = TrapGuard::save();
            let inner = (*self.inner.get()).as_mut().ok_or(AllocError)?;

            // 从 freelist 头部弹出
            if let Some(head) = inner.freepool[power] {
                // debug: freepool 头必须是 DRAM 内的合法地址——否则 free list
                // 已被覆写（越界写/use-after-free 特征），读它必崩。提前报出
                // size class 与调用点，而非事后在错误地址上 page fault。
                #[cfg(debug_assertions)]
                {
                    let cfg = crate::memory::platform::get();
                    let a = head.as_ptr() as usize;
                    if !(cfg.dram_base..cfg.dram_base + cfg.dram_size).contains(&a) {
                        panic!(
                            "block allocator: freelist head corrupted — power {power}, head {head:?} ({a:#x})"
                        );
                    }
                }
                let next = head.cast::<Option<NonNull<u8>>>().read();
                inner.freepool[power] = next;
                inner.increase_used(head, power);

                debug!("address {:?}, power {} allocated", head, power);

                return Ok(NonNull::slice_from_raw_parts(head, block_size));
            }

            inner.refill(power)
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let power = block_power(layout);

        // SAFETY: TrapGuard 关中断防止同核重入；per-hart 保证不被其他核访问。
        unsafe {
            let _trap = TrapGuard::save();
            let Some(inner) = (*self.inner.get()).as_mut() else {
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
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
            inner.freepool[power] = Some(ptr);

            debug!("address {:?}, power {} deallocated", ptr, power);

            // 该 pool 在用数 -1，归零时整页归还
            inner.decrease_used(ptr, power);
        }
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

    fn init(&mut self) {
        self.freepool.resize_with(MAX_POWER + 1, || None);
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
                HEAP_LIVE_PAGES.fetch_sub(1, Ordering::Relaxed);
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
            HEAP_LIVE_PAGES.fetch_sub(1, Ordering::Relaxed);
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

            if power < MAX_POWER {
                // 多 block 页：页头 8 字节存 used 计数，block 从 offset 8 开始
                *(base as *mut usize) = 1;
                let usable = base + 8;
                let block_nums = (PAGE_SIZE - 8) / block_size;
                link_blocks(usable, block_nums, block_size);

                let first = NonNull::new_unchecked(usable as *mut u8);
                self.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
                Ok(NonNull::slice_from_raw_parts(first, block_size))
            } else {
                // 整页单 block：无页头
                link_blocks(base, 1, block_size);

                let first = NonNull::new_unchecked(base as *mut u8);
                self.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
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

static BLOCK_ALLOCATORS: OnceLock<&'static [BlockAllocator]> = OnceLock::new();

/// 内核堆当前从 FRAME_ALLOCATOR 持有的页数（debug 统计用；release 下恒 0）。
///
/// `refill` 每取一页 +1，整页归还（`decrease_used` 两条路径）各 -1。关机时仍
/// 持有的页是堆的常驻支撑内存（任务帧之外），`check_baseline` 需扣除——否则
/// 把良性的堆页误报为任务帧泄漏。
#[cfg(debug_assertions)]
static HEAP_LIVE_PAGES: AtomicUsize = AtomicUsize::new(0);

/// 堆持有的物理页数（debug 统计用；仅 debug 构建存在）。
#[cfg(debug_assertions)]
pub(crate) fn live_pages() -> usize {
    HEAP_LIVE_PAGES.load(Ordering::Relaxed)
}

pub fn allocator() -> &'static dyn Allocator {
    let hart = unsafe { crate::memory::arch::hart_id() };
    let allocators = BLOCK_ALLOCATORS
        .get()
        .expect("block allocator not initialized");
    &allocators[hart.min(allocators.len() - 1)]
}

pub fn init() {
    let n = crate::memory::platform::get().hart_count;
    let mut v: Vec<BlockAllocator> = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(BlockAllocator::new());
    }
    let allocators = Box::leak(v.into_boxed_slice());
    for allocator in allocators.iter() {
        allocator.init();
    }
    if BLOCK_ALLOCATORS.set(allocators).is_err() {
        warn!("block allocator already initialized (init called more than once)");
    }
}
