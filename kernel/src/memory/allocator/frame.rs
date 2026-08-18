use crate::machine;
use crate::memory::PAGE_SIZE;
use core::ptr::NonNull;
use erra::ResultExt;
use log::debug;

use alloc::{
    alloc::{AllocError, Allocator},
    vec::Vec,
};

use crate::{
    lock::SpinLock,
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
    inner: SpinLock<Option<FrameInner>>,
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            inner: SpinLock::new(None),
        }
    }

    /// 初始化 frame 分配器：分配元数据 Vec（经 bump），确定基址，构建 frame freelist。
    ///
    /// 必须在所有 bump 分配（包括 block::init）之后调用，因为基址取自
    /// `bump::frontier()`——确保 frame 的 Link 节点不被后续 bump 覆盖。
    pub fn init(&self) -> Result<(), InitError> {
        let mut guard = self.inner.lock();
        let mut inner = FrameInner::new();
        inner.init()?;
        guard.replace(inner);
        Ok(())
    }

    /// 在途（未归还）物理帧数（debug 统计用；release 下恒 0）。
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub fn outstanding(&self) -> usize {
        #[cfg(debug_assertions)]
        {
            self.inner
                .lock()
                .as_ref()
                .map(|f| f.outstanding)
                .unwrap_or(0)
        }
    }
}

/// 内核持久帧基线（record_baseline 记录；check_baseline 断言回落）。
#[cfg(debug_assertions)]
static FRAME_BASELINE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// 记录基线时内核堆持有的页数——check_baseline 用它把堆支撑页从在途帧中
/// 扣除，得到真正的任务帧泄漏量。须记**差值**而非直接扣当前堆页：基线本身
/// 已含 space::init 期间 refill 的堆页，直接扣会双扣、掩盖真实泄漏。
#[cfg(debug_assertions)]
static HEAP_BASELINE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// 记录内核持久帧基线（在 spawn 用户任务**之前**调用）——此后在途帧应只增
/// 用户任务所有 + 内核堆支撑页，关机时用户任务帧全部归还。若断言触发 =
/// 任务地址空间/栈所有权 Drop 有泄漏。
pub fn record_baseline() {
    #[cfg(debug_assertions)]
    {
        FRAME_BASELINE.store(
            FRAME_ALLOCATOR.outstanding(),
            core::sync::atomic::Ordering::Relaxed,
        );
        HEAP_BASELINE.store(
            crate::memory::allocator::block::live_pages(),
            core::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// 断言关机时任务帧已全部归还（在途帧 = 内核持久帧 + 内核堆支撑页）。
///
/// block.rs 惰性 refill 的堆页是良性常驻支撑内存（任务之外），扣除后才得到
/// 真正的任务帧泄漏量。
pub fn check_baseline() {
    #[cfg(debug_assertions)]
    {
        let now = FRAME_ALLOCATOR.outstanding();
        let base = FRAME_BASELINE.load(core::sync::atomic::Ordering::Relaxed);
        let heap_base = HEAP_BASELINE.load(core::sync::atomic::Ordering::Relaxed);
        let heap_now = crate::memory::allocator::block::live_pages();
        // 内核持久帧 = 基线在途帧 − 基线时的堆页；关机在途帧 = 内核持久帧 + 当前堆页 + 任务帧。
        let expected = base.saturating_sub(heap_base) + heap_now;
        let leaked = now.saturating_sub(expected);
        assert_eq!(
            leaked, 0,
            "task frames leaked at shutdown: outstanding {now} = kernel+heap {expected} + leaked {leaked}"
        );
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
        let frame = guard.as_mut().ok_or(AllocError)?;

        let index = unsafe { frame.split_block(power) }.ok_or(AllocError)?;
        let addr = frame.frame_addr(index) as *mut u8;
        #[cfg(debug_assertions)]
        {
            frame.outstanding += 1;
        }

        debug!(
            "address {:?}, frame index {}, power {} allocated",
            addr, index, power
        );

        Ok(NonNull::slice_from_raw_parts(
            NonNull::new(addr).ok_or(AllocError)?,
            size,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        unsafe {
            let mut guard = self.inner.lock();
            let Some(frame) = guard.as_mut() else { return };
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let addr = ptr.addr().get();
            let index = frame.frame_index(addr);

            // debug: double-free 检测——pagemeta 已标 free 的帧再释放说明
            // 帧被释放两次（或归还未分配的地址），会破坏 frame 合并。
            #[cfg(debug_assertions)]
            {
                if frame.pagemeta[index].as_ref().is_some_and(|m| m.free) {
                    panic!(
                        "frame allocator: double free of index {index} (addr {addr:#x}, power {power})"
                    );
                }
            }

            frame.merge_block(index, power);
            #[cfg(debug_assertions)]
            {
                frame.outstanding = frame.outstanding.saturating_sub(1);
            }

            debug!(
                "address {:?}, frame index {}, power {} deallocated",
                addr, index, power
            );
        }
    }
}

struct FrameInner {
    freelist: Vec<Option<NonNull<Link>>>,
    pagemeta: Vec<Option<Meta>>,
    base: usize,
    edge: usize,
    /// 在途（未归还）物理帧数 — debug 断言用：关机时须回落到内核基线，
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

            // debug: freelist 头必须是 DRAM 内的合法地址——否则 free list 已被覆写
            // （越界写/use-after-free 特征），读它必崩。提前报出 size class 与
            // 调用点，而非事后在错误地址上 page fault。
            #[cfg(debug_assertions)]
            {
                let m = machine::get();
                let a = head.as_ptr() as usize;
                if !m.free.range().contains(&a) {
                    panic!(
                        "frame allocator: freelist head corrupted — power {power}, head {head:?} ({a:#x})"
                    );
                }
            }

            let addr = head.addr().get();
            let index = self.frame_index(addr);

            // debug: pagemeta index 越界检查（同 push_link）
            #[cfg(debug_assertions)]
            assert!(
                index < self.pagemeta.len(),
                "frame allocator: pop_link index {index} out of range (pagemeta len {})",
                self.pagemeta.len()
            );

            // debug: pop 出的帧必须是 free 的（pagemeta 校验）——分配到在用帧
            // 说明 frame 元数据与 freelist 不一致（重叠分配，两个持有者共享一帧）。
            #[cfg(debug_assertions)]
            if !self.pagemeta[index].as_ref().is_some_and(|m| m.free) {
                panic!(
                    "frame allocator: allocated non-free frame — index {index}, addr {addr:#x}, power {power}"
                );
            }

            let next = head.read().next;
            self.freelist[power] = next;
            if let Some(n) = next {
                // debug: next 节点也必须是 DRAM 内合法地址（head 的 Link 内容可能
                // 已被覆写——free 块被误用为数据页的特征）。
                #[cfg(debug_assertions)]
                {
                    let m = machine::get();
                    let a = n.as_ptr() as usize;
                    if !m.free.range().contains(&a) {
                        panic!(
                            "frame allocator: freelist next corrupted — power {power}, head {head:?}, next {a:#x}"
                        );
                    }
                }
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
            // debug: power 越界 = layout 计算错，写 freelist[power] 会破坏
            // 相邻内存（pagemeta 数组）——先拦下。
            #[cfg(debug_assertions)]
            assert!(
                power < self.freelist.len(),
                "frame allocator: push_link power {power} out of range (freelist len {})",
                self.freelist.len()
            );
            // debug: pagemeta index 越界 = 归还/拆分出 frame 区外的地址，
            // 写 pagemeta 会破坏相邻内存（freelist 数组）——先拦下。
            #[cfg(debug_assertions)]
            assert!(
                index < self.pagemeta.len(),
                "frame allocator: push_link index {index} out of range (pagemeta len {})",
                self.pagemeta.len()
            );
            // debug: 帧已在 freelist 再 push = 双重入链（同一帧两个 freelist
            // 条目 → 重叠分配）。遍历当前 order 链表核对（不依赖 pagemeta）。
            #[cfg(debug_assertions)]
            {
                let target = self.frame_addr(index);
                let mut cur = self.freelist[power];
                while let Some(node) = cur {
                    if node.as_ptr() as usize == target {
                        panic!(
                            "frame allocator: double push of index {index} (addr {target:#x}, power {power})"
                        );
                    }
                    cur = node.read().next;
                }
            }

            let addr = NonNull::new_unchecked(self.frame_addr(index) as *mut Link);
            addr.write(Link::new(None, self.freelist[power]));

            if let Some(head) = self.freelist[power] {
                // debug: 链表头必须是 DRAM 内合法地址（读 head 的 prev 前）。
                #[cfg(debug_assertions)]
                {
                    let m = machine::get();
                    let a = head.as_ptr() as usize;
                    if !m.free.range().contains(&a) {
                        panic!(
                            "frame allocator: push_link head corrupted — power {power}, head {head:?} ({a:#x})"
                        );
                    }
                }
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
            // debug: 被摘除的 Link 节点地址必须合法（读其 prev/next 前）。
            #[cfg(debug_assertions)]
            {
                let m = machine::get();
                let a = addr as usize;
                if !m.free.range().contains(&a) {
                    panic!(
                        "frame allocator: remove_link addr corrupted — power {power}, addr {a:#x}"
                    );
                }
                // 目标必须确实在 freelist[power] 链中——否则跨 order 交叉摘除
                // 会破坏链表（同帧被两个 order 引用时的症状）。
                let mut cur = self.freelist[power];
                let mut found = false;
                while let Some(node) = cur {
                    if node.as_ptr() as usize == a {
                        found = true;
                        break;
                    }
                    cur = node.read().next;
                }
                if !found {
                    panic!(
                        "frame allocator: remove_link target {a:#x} (index {index}) not in freelist[{power}]"
                    );
                }
            }
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

pub(crate) static FRAME_ALLOCATOR: FrameAllocator = FrameAllocator::new();

pub fn allocator() -> &'static dyn Allocator {
    &FRAME_ALLOCATOR
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
    FRAME_ALLOCATOR
        .init()
        .annotate("initializing frame allocator")
}
