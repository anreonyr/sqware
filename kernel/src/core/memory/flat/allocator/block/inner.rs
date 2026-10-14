use core::alloc::{AllocError, Layout};
use core::ptr::NonNull;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::flat::allocator::frame;
use crate::machine::sync::SpinLock;

use super::cell::Cell;
use super::pool::Pool;
use super::pump::Pump;
use super::tally::Tally;
use super::{MAX_POWER, MIN_POWER};

pub(super) struct BlockInner {
    id: usize,
    tally: &'static Tally,
    pool: SpinLock<Pool>,
    pump: [SpinLock<Pump>; MAX_POWER + 1],
}

impl BlockInner {
    pub(super) fn new(id: usize, tally: &'static Tally, pool: Pool) -> BlockInner {
        BlockInner {
            id,
            tally,
            pool: SpinLock::new(pool),
            pump: core::array::from_fn(|_| SpinLock::new(Pump::new())),
        }
    }

    pub(super) fn pull(&self, power: usize) -> Option<usize> {
        self.suck();
        let mut g = self.pool.lock();
        let inner = &mut *g;
        if let Some(head) = inner.freepool[power] {
            let page = head.as_ptr() as usize & !(PAGE_SIZE - 1);
            if inner.spare[power] == Some(page) {
                inner.spare[power] = None;
            }
            let next = unsafe { head.cast::<Option<NonNull<u8>>>().read() };
            inner.freepool[power] = next;
            self.tally.inc_used(page);

            return Some(head.as_ptr() as usize);
        }
        let first = self.prime(inner, power).ok()?;
        Some(first.as_ptr() as usize)
    }

    fn prime(&self, inner: &mut Pool, power: usize) -> Result<NonNull<u8>, AllocError> {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let page = crate::tag!(
            Prime,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| AllocError)?
        );
        crate::core::memory::flat::allocator::statistics::record_pool_take();
        let base = page.as_ptr() as *mut u8 as usize;

        self.tally.write(base, Cell::new(self.id, power));
        let block_nums = PAGE_SIZE >> power;
        unsafe {
            {
                let block_size = 1usize << power;
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
            };
            let first = NonNull::new_unchecked(base as *mut u8);
            inner.freepool[power] = first.cast::<Option<NonNull<u8>>>().read();
            first.cast::<Option<NonNull<u8>>>().write(None);
            Ok(first)
        }
    }

    fn drain(&self, inner: &mut Pool, power: usize, page: usize) {
        let mut keep: Option<NonNull<u8>> = None;
        let mut head = inner.freepool[power];
        let mut n = 0usize;
        while let Some(node) = head {
            n += 1;
            if n > 1 << 16 {
                panic!("block allocator: drain[{power}] walk exceeded depth — cyclic chain?");
            }
            let next = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
            let addr = node.as_ptr() as usize;
            if addr >= page && addr < page + PAGE_SIZE {
            } else {
                unsafe {
                    node.cast::<Option<NonNull<u8>>>().write(keep);
                }
                keep = Some(node);
            }
            head = next;
        }
        inner.freepool[power] = keep;

        self.tally.write(page, Cell::free());

        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        unsafe {
            frame::allocator().deallocate(NonNull::new_unchecked(page as *mut u8).cast(), layout);
        }
        crate::core::memory::flat::allocator::statistics::record_pool_give();
    }

    pub(super) fn push(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pool.lock();
        let inner = &mut *g;

        unsafe {
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
        }
        inner.freepool[power] = Some(ptr);

        let page = ptr.as_ptr() as usize & !(PAGE_SIZE - 1);
        let empty = self.tally.dec_used(page);
        if empty {
            if inner.spare[power].is_none() {
                inner.spare[power] = Some(page);
            } else {
                self.drain(inner, power, page);
            }
        }
    }

    pub(super) fn feed(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pump[power].lock();
        unsafe { g.push(ptr) };
    }

    pub(super) fn suck(&self) {
        for power in MIN_POWER..=MAX_POWER {
            let head = {
                let mut g = self.pump[power].lock();
                g.take()
            };
            let mut this = head;
            let mut n = 0usize;
            while let Some(node) = this {
                n += 1;
                if n > 1 << 14 {
                    panic!(
                        "block allocator: pump[{power}] walk exceeded depth — cyclic chain (double feed?)"
                    );
                }
                let next = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
                self.push(node, power);
                this = next;
            }
        }
    }

    pub(super) fn clear(&self) {
        let mut g = self.pool.lock();
        let inner = &mut *g;
        for idx in 0..self.tally.len() {
            let m = self.tally.read_idx(idx);
            if m.owner() == Some(self.id) && m.used() == 0 {
                let frame = self.tally.frame_of(idx);
                self.drain(inner, m.power as usize, frame);
            }
        }
        inner.spare.iter_mut().for_each(|s| *s = None);
    }
}
