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

    /// 在途（未归还）物理帧数（audit 统计用；未开 audit / release 恒 0）。
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub fn outstanding(&self) -> usize {
        #[cfg(debug_assertions)]
        {
            return self.inner.lock().outstanding;
        }
        #[allow(unused)]
        0
    }
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
        let size = layout.size().max(PAGE_SIZE);
        let power = block_power(size);

        let mut guard = self.inner.lock();
        let frame = &mut *guard;

        let index = unsafe { frame.split_block(power) }.ok_or(AllocError)?;
        let addr = frame.frame_addr(index) as *mut u8;
        // 护栏事件：页金库取出（Free→held；双取出 / 活堆页泄漏进池现行）。
        super::fence::on_frame_alloc(addr as usize);
        #[cfg(debug_assertions)]
        {
            frame.outstanding += 1;
        }

        checker::log_frame_alloc(addr as usize, index, power);

        Ok(NonNull::slice_from_raw_parts(
            NonNull::new(addr).ok_or(AllocError)?,
            size,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        unsafe {
            let mut guard = self.inner.lock();
            let frame = &mut *guard;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let addr = ptr.addr().get();
            let index = frame.frame_index(addr);

            // 护栏事件：页金库存入（held→Free；存入陌生页 / 双释放现行——double-free
            // 已由 Banker::credit（DoubleCredit）覆盖）。
            super::fence::on_frame_free(addr);
            frame.merge_block(index, power);
            #[cfg(debug_assertions)]
            {
                frame.outstanding = frame.outstanding.saturating_sub(1);
            }

            checker::log_frame_dealloc(addr, index, power);
        }
    }
}

struct FrameInner {
    freelist: Vec<Option<NonNull<Link>>>,
    pagemeta: Vec<Option<Meta>>,
    base: usize,
    edge: usize,
    /// 在途（未归还）物理帧数 — audit 断言用：关机时须回落到内核基线，
    /// 证明地址空间 Drop 的所有权回收无泄漏（见 schedule::scheduler::idle）。
    /// 只做 usize 计数（锁内不得分配——Vec push 会触发分配器回调，见下）。
    #[cfg(debug_assertions)]
    outstanding: usize,
}

impl FrameInner {
    const fn new() -> Self {
        Self {
            freelist: Vec::new(),
            pagemeta: Vec::new(),
            base: 0,
            edge: 0,
            #[cfg(debug_assertions)]
            outstanding: 0,
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
        // pageown 位图已在 block::init 分配（先于本处 base 定址，见 block.rs）；
        // 帧区从此基址起，位图数组在其下方、绝无重叠。
        let max_frame = self.edge.saturating_sub(self.base) / PAGE_SIZE;
        if max_frame == 0 {
            return Err(InitError::NoFreeFrames);
        }
        let max_power = max_frame.ilog2() as usize + 1;
        self.freelist.resize_with(max_power, || None);
        self.pagemeta.resize_with(max_frame, || None);

        let mut index = 0usize;
        let mut remaining = max_frame;
        while remaining > 0 {
            let power = (index.trailing_zeros() as usize)
                .min(remaining.ilog2() as usize)
                .min(max_power - 1);
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

    // frame 索引：翻转 order 对应的位
    fn buddy_index(index: usize, power: usize) -> usize {
        index ^ (1 << power)
    }

    // 从 freelist[order] 头部弹出一个空闲块，标记为非空闲，返回帧索引。
    //
    // # Safety
    //
    // 调用者需确保 freelist[order] 的链表节点指向有效的已映射物理内存。
    unsafe fn pop_link(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let head = self.freelist[power]?;
            checker::check_dram_addr(head.as_ptr() as usize, "frame pop_link (head)");

            let addr = head.addr().get();
            let index = self.frame_index(addr);
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
            addr.write(Link::new(None, self.freelist[power]));

            if let Some(head) = self.freelist[power] {
                checker::check_dram_addr(head.as_ptr() as usize, "frame push_link (head)");
                head.read().prev = Some(addr);
            }

            self.freelist[power] = Some(addr);
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

            let index = self.pop_link(k)?;

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
            while power < self.freelist.len() {
                let buddy = Self::buddy_index(index, power);

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
                    break;
                }

                self.remove_link(buddy, power);
                // 合并后 frame 并入 index 块：清除其独立 pagemeta——残留 free
                // 标记会让后续 split/merge 把已并入大块的帧当空闲块处理
                // （frame 不变量破坏 → 同一帧双重入链 → freelist 读垃圾）。
                self.pagemeta[buddy] = None;
                index = index.min(buddy); // 合并后取较小的帧索引
                power += 1;
            }

            self.push_link(index, power);
        }
    }

    /// 帧是否在 freelist[power] 链中（遍历核对）。
    ///
    /// merge 合并 frame 前调用：pagemeta 可能残留 free 标记（frame 已并入
    /// 其它块），链中核对可避免摘除不存在的节点——这是 frame 一致性修复的
    /// 本体，release 同样生效（不是纯调试防御）。
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

/// 在途（未归还）物理帧数 —— Boot audit 交叉核对、关机基线断言用。
/// 简单包一层 `FRAME_ALLOCATOR.outstanding()`，供模块外调用（未初始化 panic）。
#[cfg(debug_assertions)]
pub(crate) fn outstanding() -> usize {
    FRAME_ALLOCATOR
        .get()
        .expect("frame allocator not initialized")
        .outstanding()
}

/// 初始化 frame 分配器。
///
/// 必须在所有 bump 分配（包括 block::init）之后调用，因为基址取自
/// `bump::frontier()`——确保 frame 的 Link 节点不被后续 bump 覆盖。
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
