use crate::memory::PAGE_SIZE;
use core::ptr::NonNull;
use erra::ResultExt;

use alloc::{
    alloc::{AllocError, Allocator},
    boxed::Box,
    vec::Vec,
};

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
    fn init() -> Result<Self, InitError> {
        let mut inner = FrameInner::new();
        inner.init()?;
        Ok(Self {
            inner: SpinLock::new_level(Level::Frame, inner),
        })
    }
}

static STALE_RM: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

fn block_power(size: usize) -> usize {
    size.max(PAGE_SIZE)
        .next_multiple_of(PAGE_SIZE)
        .next_power_of_two()
        .ilog2() as usize
        - PAGE_SIZE.ilog2() as usize
}

unsafe impl Allocator for FrameAllocator {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::assert_allocation_allowed();
        {
            let this = &self;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let mut guard = this.inner.lock();
            let frame = &mut *guard;
            let index = unsafe { frame.split_block(power) }.ok_or(AllocError)?;
            let addr = frame.frame_addr(index) as *mut u8;
            #[cfg(debug_assertions)]
            {
                let a = addr as usize;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame alloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            super::statistics::record_frame_take(index, power);
            Ok(NonNull::slice_from_raw_parts(
                NonNull::new(addr).ok_or(AllocError)?,
                size,
            ))
        }
    }

    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        unsafe {
            let mut guard = self.inner.lock();
            let frame = &mut *guard;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let addr = ptr.addr().get();
            #[cfg(debug_assertions)]
            {
                let a = addr;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame dealloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            let index = frame.frame_index(addr);
            super::statistics::record_frame_give(index);
            frame.merge_block(index, power);
        }
    }
}

fn hole_containing(holes: &[Option<(usize, usize)>], index: usize) -> Option<usize> {
    holes
        .iter()
        .flatten()
        .find(|&&(start, end)| index >= start && index < end)
        .map(|&(_, end)| end)
}

struct FrameInner {
    freelist: Vec<Option<NonNull<Link>>>,
    pagemeta: Vec<Option<Meta>>,
    base: usize,
    edge: usize,
}

impl FrameInner {
    const fn new() -> Self {
        Self {
            freelist: Vec::new(),
            pagemeta: Vec::new(),
            base: 0,
            edge: 0,
        }
    }

    fn init(&mut self) -> Result<(), InitError> {
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
        super::statistics::install_frame_kinds(max_frame)?;

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
        let holes = Self::holes(self.base, self.edge, max_frame);
        while remaining > 0 {
            if let Some(end) = hole_containing(&holes, index) {
                let skip = end - index;
                index += skip;
                remaining -= skip;
                continue;
            }
            let limit = holes
                .iter()
                .flatten()
                .map(|&(start, _)| start)
                .filter(|&start| start > index)
                .min()
                .unwrap_or(max_frame);
            let mut power = (index.trailing_zeros() as usize)
                .min(remaining.ilog2() as usize)
                .min(max_power - 1);
            while index + (1 << power) > limit {
                power -= 1;
            }
            unsafe {
                self.push_link(index, power);
            }
            index += 1 << power;
            remaining -= 1 << power;
        }
        Ok(())
    }

    fn frame_index(&self, addr: usize) -> usize {
        (addr - self.base) / PAGE_SIZE
    }

    fn frame_addr(&self, index: usize) -> usize {
        self.base + index * PAGE_SIZE
    }

    fn holes(
        base: usize,
        edge: usize,
        max_frame: usize,
    ) -> [Option<(usize, usize)>; crate::platform::machine::MAX_RESERVED] {
        let mut out = [None; crate::platform::machine::MAX_RESERVED];
        let mut n = 0;
        for r in crate::platform::machine::info().reserved.iter().flatten() {
            let (hs, he) = (r.base, r.base + r.size);
            if he <= base || hs >= edge {
                continue;
            }
            let start = (hs.max(base) - base) / PAGE_SIZE;
            let end = (he.min(edge) - base).div_ceil(PAGE_SIZE);
            out[n] = Some((start.min(max_frame), end.min(max_frame)));
            n += 1;
        }
        out[..n].sort_unstable();
        out
    }

    fn buddy_index(index: usize, power: usize) -> usize {
        index ^ (1 << power)
    }

    fn clear_head(&mut self, index: usize) {
        self.pagemeta[index] = None;
    }

    unsafe fn pull_link(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let head = self.freelist[power]?;

            let addr = head.addr().get();
            let index = self.frame_index(addr);
            if index >= self.pagemeta.len() {
                static BAD: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                let n = BAD.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 8 {
                    crate::putln!(
                        "frame freelist corrupt: pop index={index:#x} len={} power={power} ({n})",
                        self.pagemeta.len()
                    );
                }
                return None;
            }

            let next = head.read().next;
            self.freelist[power] = next;
            if let Some(n) = next {
                (*n.as_ptr()).prev = None;
            }

            self.clear_head(index);
            Some(index)
        }
    }

    unsafe fn push_link(&mut self, index: usize, power: usize) {
        unsafe {
            let addr = NonNull::new_unchecked(self.frame_addr(index) as *mut Link);
            addr.write(Link::new(None, self.freelist[power]));

            if let Some(head) = self.freelist[power] {
                (*head.as_ptr()).prev = Some(addr);
            }

            self.freelist[power] = Some(addr);
            self.pagemeta[index] = Some(Meta::new(true, power as u8));
        }
    }

    unsafe fn remove_link(&mut self, index: usize, power: usize) {
        unsafe {
            let addr = self.frame_addr(index) as *mut Link;

            let prev = (*addr).prev;
            let next = (*addr).next;

            if let Some(p) = prev {
                (*p.as_ptr()).next = next;
            } else {
                let head = self.freelist[power].map(|h| h.as_ptr() as usize);
                if head != Some(addr as usize) {
                    let n = STALE_RM.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                    if n <= 8 {
                        crate::putln!(
                            "[stale-rm] remove_link idx={index} power={power} 自称桶头但桶头={:?} next={:?}（第 {n} 次）",
                            head.map(|h| (h - self.base) / PAGE_SIZE),
                            next.map(|x| (x.as_ptr() as usize - self.base) / PAGE_SIZE)
                        );
                    }
                }
                self.freelist[power] = next;
            }
            if let Some(n) = next {
                (*n.as_ptr()).prev = prev;
            }
        }
    }

    unsafe fn split_block(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let mut k = power;
            while k < self.freelist.len() && self.freelist[k].is_none() {
                k += 1;
            }
            if k >= self.freelist.len() {
                return None;
            }

            let index = self.pull_link(k)?;

            while k > power {
                k -= 1;
                let buddy = Self::buddy_index(index, k);
                self.push_link(buddy, k);
            }

            self.pagemeta[index] = Some(Meta::new(false, power as u8));

            Some(index)
        }
    }

    unsafe fn merge_block(&mut self, mut index: usize, mut power: usize) {
        unsafe {
            self.clear_head(index);
            while power < self.freelist.len() {
                let buddy = Self::buddy_index(index, power);

                if buddy >= self.pagemeta.len() {
                    break;
                }

                if !self.pagemeta[buddy]
                    .as_ref()
                    .is_some_and(|m| m.free && m.power as usize == power)
                {
                    break;
                }
                if !self.in_freelist(buddy, power) {
                    break;
                }

                self.remove_link(buddy, power);
                self.clear_head(buddy);
                index = index.min(buddy);
                power += 1;
            }

            self.push_link(index, power);
            if !self.in_freelist(index, power) {
                static LOST: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                let n = LOST.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 8 {
                    crate::putln!(
                        "merge: pushed block NOT in chain idx={index} power={power} ({n})"
                    );
                }
            }
        }
    }

    fn in_freelist(&self, index: usize, power: usize) -> bool {
        let target = self.frame_addr(index);
        let mut cur = self.freelist[power];
        while let Some(node) = cur {
            if node.as_ptr() as usize == target {
                return true;
            }
            // SAFETY: freelist 节点恒为已释放块，头 16 字节是 Link
            cur = unsafe { node.read() }.next;
        }
        false
    }
}

static FRAME_ALLOCATOR: OnceLock<&'static FrameAllocator> = OnceLock::new();

pub fn allocator() -> &'static dyn Allocator {
    FRAME_ALLOCATOR
        .get()
        .expect("frame allocator not initialized")
}

pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        // Reserve the allocator itself before calculating the first free frame.
        let mut slot =
            Box::<FrameAllocator>::try_new_uninit().map_err(|_| InitError::OutOfMemory)?;
        slot.write(FrameAllocator::init()?);
        // SAFETY: initialization filled the reserved slot.
        let heap = Box::leak(unsafe { slot.assume_init() });
        FRAME_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing frame allocator")
}
