// spare — 后备仓分配器（日志 + panic 打印专用）：coalescing free-list，有序相邻合并。
//
// 块模型（有序单链 free-list，无 buddy 幂级）：
//   Blk = 侵入式 Link（复用 allocator::Link）+ size（本块总长含 HEADER），写块首；
//   块首 16B 对齐 ⇒ 载荷 16B 对齐（MAX_ALIGN 内请求零垫片）。链恒按地址升序——
//   pull 首适配 + 拆余、push 排序插入 + 前后邻合并，绝不重复入链。
//
// 不变·硬（贴结构）：
//   - 所有空闲块头写在本区块内，块与块首尾相接（合并后 size 恰为两邻之和）；
//   - used 记账含块头且每字节至多归属一次（合并不重复计数）；
//   - 区内指针才可释放（区外 = 主堆误还/切换窗口跨堆 drop：静默丢弃、绝不 panic）；
//   - 只支持 align ≤ MAX_ALIGN 的请求（更高对齐回 Err）。

use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use erra::ResultExt;

use crate::{
    lock::{Level, OnceLock, SpinLock},
    machine,
    memory::{
        PAGE_SIZE,
        allocator::{InitError, InitResult, Link, hybrid},
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
/// `used` 由 statistics::Stats.spare.occupied 唯一权威,本结构不再保留。
struct SpareInner {
    base: usize,
    edge: usize,
    head: Option<NonNull<Blk>>,
}

impl SpareInner {
    /// 建仓：整块区作为一个空闲块入链,记账清零。
    fn new(base: usize, edge: usize) -> Self {
        let head = unsafe { Blk::write(base, edge - base) };
        Self {
            base,
            edge,
            head: Some(head),
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
    /// **继承 blk 原链位**（prev/next 拷给 rest，邻居改指 rest）——位置不变、出入链
    /// 一次；整块给出则整体出链。曾丢继承：rest 链位（尤其 next）不拷会造成多节点
    /// 下尾部块断链（单节点时原链位为空、恰好无感，dump 多节点序列才现行）。
    fn split(&mut self, blk: NonNull<Blk>, need: usize) {
        let start = blk.as_ptr() as usize;
        // SAFETY: blk 在链中，其 size 为块首写的真实总长。
        let size = unsafe { blk.read() }.size;
        let left = size - HEADER - need;
        if left >= HEADER {
            let rest = unsafe { Blk::write(start + HEADER + need, left) };
            // SAFETY: 邻居指针与 rest 均在仓内；blk 被分配（出链）、rest 继承其链位。
            unsafe {
                let l = blk.read().link;
                (*rest.as_ptr()).link = l;
                // Link 非 Copy：从 rest 上读 Copy 成员（已持有原块 prev/next）。
                let (p, n) = ((*rest.as_ptr()).link.prev, (*rest.as_ptr()).link.next);
                match p {
                    Some(p) => (*p.as_ptr()).next = Some(rest.cast()),
                    None => self.head = Some(rest),
                }
                if let Some(n) = n {
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

/// 崩溃打印峰值预算。
pub const DUMP_BUDGET: usize = 1024 * 1024;

pub struct SpareAllocator {
    inner: SpinLock<SpareInner>,
}

impl SpareAllocator {
    /// 建仓：经 hybrid 一次性 allocate 仓容量（PAGE 对齐、整块连续），整区入链。
    ///
    /// # Errors
    ///
    /// 主堆余量不足 → [`InitError::OutOfMemory`]。
    fn init() -> Result<Self, InitError> {
        let cap = {
            let payload = crate::runtime::diagnose::trace::ring_bytes(machine::hart_count());
            (payload.next_multiple_of(MAX_ALIGN) + HEADER + DUMP_BUDGET).next_multiple_of(PAGE_SIZE)
        };
        let align = Layout::from_size_align(cap, PAGE_SIZE).map_err(|_| InitError::OutOfMemory)?;
        let chunk = hybrid::allocator()
            .allocate(align)
            .map_err(|_| InitError::OutOfMemory)?;
        let base = chunk.as_ptr() as *mut u8 as usize;
        // 持久注册表：spare 仓块（日志 + panic 打印专用）永不归还——登记以便
        // 关机逐项核 held（②）。
        #[cfg(feature = "audit")]
        crate::memory::allocator::fence::audit::register_persistent(base, "spare");
        let edge = base + chunk.len();
        Ok(Self {
            inner: SpinLock::new_level(Level::Spare, SpareInner::new(base, edge)),
        })
    }

    /// 仓总容量（edge − base 字节）。statistics::view_spare().total 派生于此。
    pub(crate) fn total_bytes(&self) -> usize {
        let g = self.inner.lock();
        g.edge - g.base
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
        super::statistics::record_spare_take(size);
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
                // 跨堆指针（切换前主堆分配、切换后 drop）：不还本仓、绝不 panic——直接丢弃。
                return;
            }
            checker::check_dram_addr(blk_addr, "spare put");
            let blk = NonNull::new_unchecked(blk_addr as *mut Blk);
            let size = blk.read().size;
            super::statistics::record_spare_give(size);
            inner.push(blk);
        }
    }
}

// 锁（!Send）按值不能直接 `OnceLock<SpareAllocator>`，存 `&'static` 引用，
// init 时 Box::leak。
static SPARE_ALLOCATOR: OnceLock<&'static SpareAllocator> = OnceLock::new();

/// 后备仓入口（统一暴露）：返回具体 `SpareAllocator`——调用方直接
/// `spare().allocate(...)`。`occupied` / `available` 走 `statistics::view_spare()`。
pub fn spare() -> &'static SpareAllocator {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
}

pub fn allocator() -> &'static dyn Allocator {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
}

/// 初始化后备仓（boot 恰好一次，hybrid 初始化之后调用）。
///
/// # Errors
///
/// 主堆余量不足（预算超可用）→ [`InitError::OutOfMemory`]。
pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = Box::leak(Box::new(SpareAllocator::init()?));
        SPARE_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing spare allocator")
}
