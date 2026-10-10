use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use erra::ResultExt;

use crate::hart;
use crate::{
    lock::{Level, OnceLock, SpinLock},
    memory::{
        PAGE_SIZE,
        allocator::{InitError, InitResult, Link, hybrid},
    },
};

const HEADER: usize = 32;
const MAX_ALIGN: usize = 16;

#[repr(C)]
struct Blk {
    link: Link,
    size: usize,
}

impl Blk {
    unsafe fn write(addr: usize, size: usize) -> NonNull<Blk> {
        let blk = unsafe { NonNull::new_unchecked(addr as *mut Blk) };
        // SAFETY: blk 指向本区块首，写入头 16 字节 Link + size 不越区
        unsafe {
            blk.write(Blk {
                link: Link::new(None, None),
                size,
            });
        }
        blk
    }

    fn payload(&self) -> usize {
        self.size - HEADER
    }

    fn next(&self) -> Option<NonNull<Blk>> {
        self.link.next.map(|n| n.cast())
    }
}

struct SpareInner {
    base: usize,
    edge: usize,
    head: Option<NonNull<Blk>>,
}

impl SpareInner {
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

    fn unlink(&mut self, blk: NonNull<Blk>) {
        // SAFETY: blk 必在链中；头 16 字节是 Link
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

    fn pull(&mut self, need: usize) -> Option<NonNull<Blk>> {
        let mut cur = self.head;
        while let Some(blk) = cur {
            // SAFETY: 链节点恒为仓内空闲块
            if unsafe { blk.read() }.payload() >= need {
                self.split(blk, need);
                return Some(blk);
            }
            cur = unsafe { blk.read() }.next();
        }
        None
    }

    fn split(&mut self, blk: NonNull<Blk>, need: usize) {
        let start = blk.as_ptr() as usize;
        // SAFETY: blk 在链中，其 size 为块首写的真实总长
        let size = unsafe { blk.read() }.size;
        let left = size - HEADER - need;
        if left >= HEADER {
            let rest = unsafe { Blk::write(start + HEADER + need, left) };
            // SAFETY: 邻居指针与 rest 均在仓内
            unsafe {
                let l = blk.read().link;
                (*rest.as_ptr()).link = l;
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
        // SAFETY: prev/cur/neighbors 均为仓内空闲块
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
        if let Some(p) = prev {
            let pend = (p.as_ptr() as usize) + unsafe { p.read() }.size;
            if pend == addr {
                // SAFETY: p、blk 相邻且均在链中
                unsafe {
                    (*p.as_ptr()).size += blk.read().size;
                }
                self.unlink(blk);
                blk = p;
            }
        }
        if let Some(n) = unsafe { blk.read() }.next() {
            let bend = (blk.as_ptr() as usize) + unsafe { blk.read() }.size;
            if bend == n.as_ptr() as usize {
                // SAFETY: blk、n 相邻且均在链中
                unsafe {
                    (*blk.as_ptr()).size += n.read().size;
                }
                self.unlink(n);
            }
        }
    }
}

pub const DUMP_BUDGET: usize = 1024 * 1024;

pub struct SpareAllocator {
    inner: SpinLock<SpareInner>,
}

impl SpareAllocator {
    fn init() -> Result<Self, InitError> {
        let cap = {
            let payload = crate::runtime::diagnose::trace::ring_bytes(hart::hart_count());
            (payload.next_multiple_of(MAX_ALIGN) + HEADER + DUMP_BUDGET).next_multiple_of(PAGE_SIZE)
        };
        let align = Layout::from_size_align(cap, PAGE_SIZE).map_err(|_| InitError::OutOfMemory)?;
        let chunk = crate::tag!(
            Spare,
            hybrid::allocator()
                .allocate(align)
                .map_err(|_| InitError::OutOfMemory)?
        );
        let base = chunk.as_ptr() as *mut u8 as usize;
        let edge = base + chunk.len();
        Ok(Self {
            inner: SpinLock::new_level(Level::Spare, SpareInner::new(base, edge)),
        })
    }

    pub(crate) fn total_bytes(&self) -> usize {
        let g = self.inner.lock();
        g.edge - g.base
    }
}

unsafe impl Allocator for SpareAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::assert_allocation_allowed();
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
        // SAFETY: blk 在链中，size 为块首真实总长
        let size = unsafe { blk.read() }.size;
        super::statistics::record_spare_take(size);
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
                return;
            }

            let blk = NonNull::new_unchecked(blk_addr as *mut Blk);
            let size = blk.read().size;
            super::statistics::record_spare_give(size);
            inner.push(blk);
        }
    }
}

static SPARE_ALLOCATOR: OnceLock<&'static SpareAllocator> = OnceLock::new();

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

pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap =
            Box::leak(Box::try_new(SpareAllocator::init()?).map_err(|_| InitError::OutOfMemory)?);
        SPARE_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing spare allocator")
}
