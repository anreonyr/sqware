// spare — 后备仓分配器（日志 + panic 打印专用）：coalescing free-list，有序相邻合并
//
// 与主堆（portal → bump/hybrid → block/frame）**互不可见**：
//   · 区 —— boot 早期从 bump 头顶 carve 一次性成仓（3 号位：bump::init 之后、
//     hybrid::init 之前），大小 = 诊断预算（spare_budget(h)：trace 环形常驻 +
//     panic 打印峰值，见 diagnose::budget）。bump 前沿越过本仓，frame/block 永不触
//     碰——崩溃现场分配器可信的唯一来源，主堆烂了也不影响本仓。
//   · 用 —— 常态**显式调用**（trace 环形、health 演练指名道姓取 `spare::allocator()`）；
//     崩溃 = 报警核把门户后端**无锁切换**到本仓（portal::switch(Backend::Spare)——原子
//     store，不取任何锁，panic 恰在持主堆锁现场也不卡死），转储渲染的分配整个
//     落仓（含未来 stanza 模型的隐式分配）。自持 SpinLock（Level::Spare）：
//     持锁者恒为瞬时上下文，无死锁环。
//   · 失 —— 预算即契约：health 验收断言 ring 常驻后余量 ≥ DUMP_BUDGET，溢出演练
//     证明失败路径返回 Err（不 panic，调用方映射）。
//
// 块模型（有序单链 free-list，无 buddy 幂级——仓小、碎片随合并消解）：
//   Blk = 侵入式 Link（复用 allocator::Link）+ size（本块总长含 HEADER），写块首；
//   块首 16B 对齐 ⇒ 载荷 16B 对齐（MAX_ALIGN 内请求零垫片）。链恒按地址升序——
//   pull 首适配 + 拆余、push 排序插入 + 前后邻合并，绝不重复入链。
//
// 不变·硬（贴结构）：
//   - 所有空闲块头写在本区块内，块与块首尾相接（合并后 size 恰为两邻之和）；
//   - used 记账含块头且每字节至多归属一次（合并不重复计数）；peak 单调；
//   - 区内指针才可释放（区外 = 主堆误还，debug_assert 现行）；
//   - 只支持 align ≤ MAX_ALIGN 的请求（更高对齐回 Err）——崩溃打印的
//     Vec/String/Box 均 ≤ 16B 对齐。
//
// 命名：spare（后备仓）与 block 的备页 spare 同一隐喻延伸（备件储备）；块操作
// pull/push（池内拉/推，同 block 家族动词）；合并 merge；统计 used/peak/remaining。

use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use erra::ResultExt;

use crate::machine;
use crate::{
    lock::{Level, OnceLock, SpinLock},
    memory::{
        PAGE_SIZE,
        allocator::{InitError, InitResult, Link, bump},
    },
};

use super::fence::checker;

/// 块头字节数：Link(16) + size(8) + 对齐垫(8) → 块首 16B 对齐 ⇒ 载荷 16B 对齐。
const HEADER: usize = 32;
/// 支持的请求对齐上限（16B；超限回 Err，不 panic）。
const MAX_ALIGN: usize = 16;

/// 空闲块头（写在空闲块首）：侵入式双向链节点 + 本块总长（含 HEADER）。
#[repr(C)]
struct Blk {
    link: Link,
    size: usize,
}

impl Blk {
    /// 在 `addr` 写一个初空链位、总长 `size` 的块头，返回其指针。
    ///
    /// # Safety
    ///
    /// `addr` 必须是仓内空闲区、16B 对齐，且后续 `size` 字节空闲可用。
    unsafe fn write(addr: usize, size: usize) -> NonNull<Blk> {
        // SAFETY: 调用方保证 addr 为仓内空闲区、16B 对齐、size 字节可用。
        let blk = unsafe { NonNull::new_unchecked(addr as *mut Blk) };
        // SAFETY: blk 指向本区块首，写入头 16 字节 Link + size 不越区。
        unsafe {
            blk.write(Blk {
                link: Link::new(None, None),
                size,
            });
        }
        blk
    }

    /// 载荷字节数（总长 − 块头）。
    fn payload(&self) -> usize {
        self.size - HEADER
    }

    /// 链后继（Blk 类型化）。
    fn next(&self) -> Option<NonNull<Blk>> {
        self.link.next.map(|n| n.cast())
    }
}

/// 仓内状态（SpinLock 壳内；Level::Spare，见 lock::depend）。
struct SpareInner {
    base: usize,
    edge: usize,
    head: Option<NonNull<Blk>>,
    used: usize,
    peak: usize,
}

impl SpareInner {
    /// 建仓：整块区作为一个空闲块入链，记账清零。
    fn new(base: usize, edge: usize) -> Self {
        let head = unsafe { Blk::write(base, edge - base) };
        Self {
            base,
            edge,
            head: Some(head),
            used: 0,
            peak: 0,
        }
    }

    fn owns(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.edge
    }

    /// 出链（链位由 blk 头内 link 给出；邻居/链头改接）。
    fn unlink(&mut self, blk: NonNull<Blk>) {
        // SAFETY: blk 必在链中（调用方保证）；头 16 字节是 Link，读改写仅在仓内空闲块。
        unsafe {
            let l = blk.read().link;
            match l.prev {
                Some(p) => (*p.as_ptr()).next = l.next,
                None => self.head = l.next.map(|n| n.cast()),
            }
            if let Some(n) = l.next {
                (*n.as_ptr()).prev = l.prev;
            }
        }
    }

    /// 取块：有序首适配 ≥ need 的空闲块；需要则拆出剩余块留在链中（继承原链位）。
    fn pull(&mut self, need: usize) -> Option<NonNull<Blk>> {
        let mut cur = self.head;
        while let Some(blk) = cur {
            // SAFETY: 链节点恒为仓内空闲块（不变·硬 1）；头 16 字节是 Link。
            if unsafe { blk.read() }.payload() >= need {
                self.split(blk, need);
                return Some(blk);
            }
            cur = unsafe { blk.read() }.next();
        }
        None
    }

    /// 拆块：前 `need`（16B 倍数）作分配区留在 blk，剩余（≥ HEADER）另成一空闲块
    /// 继承 blk 原链位——位置不变、出入链一次，整块给出则整体出链。
    fn split(&mut self, blk: NonNull<Blk>, need: usize) {
        let start = blk.as_ptr() as usize;
        // SAFETY: blk 在链中，其 size 为块首写的真实总长。
        let size = unsafe { blk.read() }.size;
        let left = size - HEADER - need;
        if left >= HEADER {
            let rest = unsafe { Blk::write(start + HEADER + need, left) };
            // SAFETY: 邻居指针与 rest 均在仓内；head 换首、邻居改接。
            unsafe {
                let l = blk.read().link;
                match l.prev {
                    Some(p) => (*p.as_ptr()).next = Some(rest.cast()),
                    None => self.head = Some(rest),
                }
                if let Some(n) = l.next {
                    (*n.as_ptr()).prev = Some(rest.cast());
                }
                (*blk.as_ptr()).size = HEADER + need;
            }
        } else {
            self.unlink(blk);
        }
    }

    /// 归块：按地址升序插入，随后并入前邻、再并入后邻（首尾相接即合并）。
    fn push(&mut self, mut blk: NonNull<Blk>) {
        let addr = blk.as_ptr() as usize;
        let mut prev: Option<NonNull<Blk>> = None;
        let mut cur = self.head;
        while let Some(n) = cur {
            if n.as_ptr() as usize > addr {
                break;
            }
            prev = Some(n);
            cur = unsafe { n.read() }.next();
        }
        // 插在 prev 与 cur 之间（有序性保持）。
        // SAFETY: prev/cur/neighbors 均为仓内空闲块；blk 尚未入链，写其头安全。
        unsafe {
            (*blk.as_ptr()).link.prev = prev.map(|p| p.cast());
            (*blk.as_ptr()).link.next = cur.map(|n| n.cast());
            match prev {
                Some(p) => (*p.as_ptr()).link.next = Some(blk.cast()),
                None => self.head = Some(blk),
            }
            if let Some(n) = cur {
                (*n.as_ptr()).link.prev = Some(blk.cast());
            }
        }
        // 前邻相邻 → blk 并入 prev（blk 头作废）。
        if let Some(p) = prev {
            let pend = (p.as_ptr() as usize) + unsafe { p.read() }.size;
            if pend == addr {
                // SAFETY: p、blk 相邻且均在链中；并入后 p.size = 两邻之和。
                unsafe {
                    (*p.as_ptr()).size += blk.read().size;
                }
                self.unlink(blk);
                blk = p;
            }
        }
        // 后邻相邻 → next 并入 blk。
        if let Some(n) = unsafe { blk.read() }.next() {
            let bend = (blk.as_ptr() as usize) + unsafe { blk.read() }.size;
            if bend == n.as_ptr() as usize {
                // SAFETY: blk、n 相邻且均在链中；并入后 blk.size = 两邻之和。
                unsafe {
                    (*blk.as_ptr()).size += n.read().size;
                }
                self.unlink(n);
            }
        }
    }
}

/// 给定载荷与预留所需仓容：载荷（16B 取整）+ 块头 + 预留，页对齐后整取。
///
/// 供诊断预算（diagnose::budget::spare_budget）与验收（health::spare）共用的
/// 开销公式——预算与记账同源，remaining 恒 ≥ 预留。
pub fn region_size(payload: usize, reserve: usize) -> usize {
    (payload.next_multiple_of(MAX_ALIGN) + HEADER + reserve).next_multiple_of(PAGE_SIZE)
}

pub(crate) struct SpareAllocator {
    inner: SpinLock<SpareInner>,
}

impl SpareAllocator {
    /// 建仓：从 bump 头顶 carve 诊断预算（PAGE 对齐、线性连续），整区入链。
    ///
    /// 容量不显式注入：按 `machine::hart_count()` 经诊断预算
    /// （diagnose::budget::spare_budget = trace 环形常驻 + panic 打印峰值）自查——
    /// 同 bump 读 `machine::info().free` 的查源习惯。前置：bump::init 之后、
    /// hybrid::init 之前（frame 基址 = bump 前沿，晚于本处会切到 frame 已认领的区）。
    ///
    /// # Errors
    ///
    /// bump 余量不足 → [`InitError::OutOfMemory`]。
    fn init() -> Result<Self, InitError> {
        let cap = crate::runtime::diagnose::budget::spare_budget(machine::hart_count());
        let align = Layout::from_size_align(cap, PAGE_SIZE).map_err(|_| InitError::OutOfMemory)?;
        let chunk = bump::allocator()
            .allocate(align)
            .map_err(|_| InitError::OutOfMemory)?;
        let base = chunk.as_ptr() as *mut u8 as usize;
        let edge = base + chunk.len();
        Ok(Self {
            inner: SpinLock::new_level(Level::Spare, SpareInner::new(base, edge)),
        })
    }

    fn used(&self) -> usize {
        self.inner.lock().used
    }

    fn peak(&self) -> usize {
        self.inner.lock().peak
    }

    fn remaining(&self) -> usize {
        let inner = self.inner.lock();
        (inner.edge - inner.base) - inner.used
    }
}

unsafe impl Allocator for SpareAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.align() > MAX_ALIGN {
            return Err(AllocError);
        }
        let need = layout.size().max(1).next_multiple_of(MAX_ALIGN);
        let mut guard = self.inner.lock();
        let inner = &mut *guard;
        let Some(blk) = inner.pull(need) else {
            return Err(AllocError);
        };
        let addr = blk.as_ptr() as usize;
        // SAFETY: blk 在链中，size 为块首真实总长（split 后 = HEADER + need）。
        let size = unsafe { blk.read() }.size;
        checker::check_dram_addr(addr, "spare pull");
        inner.used += size;
        inner.peak = inner.peak.max(inner.used);
        // SAFETY: addr + HEADER 为仓内空闲块载荷首（16B 对齐，块已出链）。
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new((addr + HEADER) as *mut u8).ok_or(AllocError)?,
            layout.size(),
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        unsafe {
            let blk_addr = ptr.as_ptr() as usize - HEADER;
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            if !inner.owns(blk_addr) {
                // 区外指针（主堆对象误还）：不释放、不腐坏——崩溃转储不灭尾主堆
                // Arc（不变量），此路径 = 不变量被破坏：debug 当场报警，release
                // 静默丢弃（宁漏不烂，转储继续、halt 照常收敛）。
                debug_assert!(false, "spare: deallocate outside arena");
                return;
            }
            checker::check_dram_addr(blk_addr, "spare put");
            let blk = NonNull::new_unchecked(blk_addr as *mut Blk);
            let size = blk.read().size;
            inner.used = inner.used.saturating_sub(size);
            inner.push(blk);
        }
    }
}

// 锁（!Send）按值不能直接 `OnceLock<SpareAllocator>`，存 `&'static` 引用，
// init 时 Box::leak（同 frame）。
static SPARE_ALLOCATOR: OnceLock<&'static SpareAllocator> = OnceLock::new();

pub fn allocator() -> &'static dyn Allocator {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
}

/// 在册字节（含块头，常驻 ring 计入）。
pub fn used() -> usize {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
        .used()
}

/// 历史最高在册字节（预算验收断言峰值用）。
pub fn peak() -> usize {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
        .peak()
}

/// 余量字节（仓容 − 在册；诊断打印的可用预算）。
pub fn remaining() -> usize {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
        .remaining()
}

/// 初始化后备仓（boot 恰好一次，allocator::init 内 3 号位调用）。
///
/// 必须在所有 bump 分配之前 carve（bump 前沿单调前进，晚取会从 frame 已认领的
/// 区里切块）。区为自由 DRAM 前切片，恒等可见、崩溃现场可读。
///
/// # Errors
///
/// bump 余量不足（预算超空闲区）→ [`InitError::OutOfMemory`]。
pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = Box::leak(Box::new(SpareAllocator::init()?));
        SPARE_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing spare allocator")
}

