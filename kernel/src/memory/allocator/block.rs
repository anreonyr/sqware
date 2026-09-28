use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::vec::Vec;
use erra::ResultExt;

use crate::hart;
use crate::memory::PAGE_SIZE;
use crate::platform::machine;
use crate::{
    lock::{Level, OnceLock, SpinLock},
    memory::allocator::{InitError, InitResult, bump, frame},
};

const MIN_POWER: usize = 3;
const MAX_POWER: usize = (PAGE_SIZE / 2).ilog2() as usize;
const PAGE_SHIFT: usize = PAGE_SIZE.ilog2() as usize;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Meta {
    owner: Option<u8>,
    power: u8,
    used: u16,
}

impl Meta {
    const fn free() -> Self {
        Self {
            owner: None,
            power: 0,
            used: 0,
        }
    }

    const fn new(owner: usize, power: usize) -> Self {
        Self {
            owner: Some(owner as u8),
            power: power as u8,
            used: 1,
        }
    }

    fn owner(self) -> Option<usize> {
        self.owner.map(|o| o as usize)
    }

    fn used(self) -> u16 {
        self.used
    }

    fn inc_used(self) -> Self {
        Self {
            used: self.used.saturating_add(1),
            ..self
        }
    }

    fn dec_used(self) -> (Self, bool) {
        let used = self.used.saturating_sub(1);
        (Self { used, ..self }, used == 0)
    }
}

struct Tally {
    base: usize,
    cells: *mut Meta,
    len: usize,
    lock: SpinLock<()>,
}

// SAFETY: cells 指向 'static bump 内存；全部访问经 lock 串行
unsafe impl Send for Tally {}
unsafe impl Sync for Tally {}

impl Tally {
    fn new(base: usize, cells: *mut Meta, len: usize) -> Self {
        Self {
            base,
            cells,
            len,
            lock: SpinLock::new_level(Level::Tally, ()),
        }
    }

    fn idx(&self, pa: usize) -> Option<usize> {
        let page = pa & !(PAGE_SIZE - 1);
        let idx = page.checked_sub(self.base)? >> PAGE_SHIFT;
        (idx < self.len).then_some(idx)
    }

    fn frame_of(&self, idx: usize) -> usize {
        self.base + (idx << PAGE_SHIFT)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn owner_of(&self, pa: usize) -> Option<usize> {
        let _g = self.lock.lock();
        // SAFETY: idx 通过上下界检查；lock 串行
        let m = unsafe { self.cells.add(self.idx(pa)?).read() };
        m.owner()
    }

    fn read_idx(&self, idx: usize) -> Meta {
        let _g = self.lock.lock();
        // SAFETY: idx < len 已由调用方保证
        unsafe { self.cells.add(idx).read() }
    }

    fn write(&self, page: usize, m: Meta) {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行
        unsafe {
            self.cells.add(idx).write(m);
        }
    }

    fn inc_used(&self, page: usize) -> Meta {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行，RMW 原子
        unsafe {
            let mut m = self.cells.add(idx).read();
            m = m.inc_used();
            self.cells.add(idx).write(m);
            m
        }
    }

    fn dec_used(&self, page: usize) -> (Meta, bool) {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        // SAFETY: idx 已检查；lock 串行，RMW 原子
        unsafe {
            let m = self.cells.add(idx).read();
            let (m, empty) = m.dec_used();
            self.cells.add(idx).write(m);
            (m, empty)
        }
    }
}

struct Pump {
    head: Option<NonNull<u8>>,
    len: usize,
}

impl Pump {
    const fn new() -> Self {
        Self { head: None, len: 0 }
    }

    unsafe fn push(&mut self, ptr: NonNull<u8>) {
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>().write(self.head);
            self.head = Some(ptr);
            self.len += 1;
        }
    }

    fn take(&mut self) -> Option<NonNull<u8>> {
        let head = self.head.take();
        self.len = 0;
        head
    }
}

pub(crate) struct BlockAllocator {
    blocks: &'static [BlockInner],
    tally: &'static Tally,
}

impl BlockAllocator {
    pub(crate) fn own(&self, pa: usize) -> Option<usize> {
        self.tally.owner_of(pa)
    }

    fn init() -> Result<Self, InitError> {
        let nodes = hart::hart_count();
        assert!(nodes > 0, "block init: no harts");
        let m = machine::info();

        let tally = {
            let meta_len = m.free.size.div_ceil(PAGE_SIZE);
            let meta = bump::allocator()
                .allocate(
                    core::alloc::Layout::from_size_align(
                        meta_len * core::mem::size_of::<Meta>(),
                        core::mem::align_of::<Meta>(),
                    )
                    .unwrap(),
                )
                .map_err(|_| InitError::OutOfMemory)?;
            let cells = meta.as_ptr() as *mut Meta;
            unsafe {
                for i in 0..meta_len {
                    cells.add(i).write(Meta::free());
                }
            }
            Box::leak(Box::new(Tally::new(m.free.base, cells, meta_len)))
        };
        super::statistics::install_block_kinds(m.free.base, m.free.size)?;

        let mut pools = Vec::new();
        for i in 0..nodes {
            let mut pool = Pool::new();
            pool.init()?;
            pools.push(BlockInner::new(i, tally, pool));
        }

        Ok(BlockAllocator {
            blocks: Box::leak(pools.into_boxed_slice()),
            tally,
        })
    }
}

unsafe impl Allocator for BlockAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let power = layout
            .size()
            .max(1usize << MIN_POWER)
            .next_power_of_two()
            .ilog2() as usize;
        if power > MAX_POWER || layout.align() > (1usize << power) {
            return Err(AllocError);
        }
        let me = hart::hart_id();
        let pool = &self.blocks[me.get()];
        let addr = pool.pull(power).ok_or(AllocError)?;
        super::statistics::record_block_take(addr, power);

        Ok(NonNull::slice_from_raw_parts(
            unsafe { NonNull::new_unchecked(addr as *mut u8) },
            layout.size(),
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let power = layout
            .size()
            .max(1usize << MIN_POWER)
            .next_power_of_two()
            .ilog2() as usize;
        let pa = ptr.addr().get();
        let Some(home) = self.own(pa) else { return };
        super::statistics::record_block_give(pa, power);

        let me = hart::hart_id();
        let pool = &self.blocks[home];
        if home == me.get() {
            pool.push(ptr, power);
        } else {
            pool.feed(ptr, power);
        }
    }
}

pub(crate) struct BlockInner {
    id: usize,
    tally: &'static Tally,
    pool: SpinLock<Pool>,
    pump: [SpinLock<Pump>; MAX_POWER + 1],
}

impl BlockInner {
    fn new(id: usize, tally: &'static Tally, pool: Pool) -> BlockInner {
        BlockInner {
            id,
            tally,
            pool: SpinLock::new(pool),
            pump: core::array::from_fn(|_| SpinLock::new(Pump::new())),
        }
    }

    fn meta_put(&self, page: usize, m: Meta) {
        self.tally.write(page, m);
    }

    fn pull(&self, power: usize) -> Option<usize> {
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
        super::statistics::record_pool_take();
        let base = page.as_ptr() as *mut u8 as usize;

        self.meta_put(base, Meta::new(self.id, power));
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
            // SAFETY: 链中块均已在 freepool（free 状态），首字为 next 指针
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

        self.meta_put(page, Meta::free());

        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        unsafe {
            frame::allocator().deallocate(NonNull::new_unchecked(page as *mut u8).cast(), layout);
        }
        super::statistics::record_pool_give();
    }

    fn push(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pool.lock();
        let inner = &mut *g;

        unsafe {
            ptr.cast::<Option<NonNull<u8>>>()
                .write(inner.freepool[power]);
        }
        inner.freepool[power] = Some(ptr);

        let page = ptr.as_ptr() as usize & !(PAGE_SIZE - 1);
        let (_, empty) = self.tally.dec_used(page);
        if empty {
            if inner.spare[power].is_none() {
                inner.spare[power] = Some(page);
            } else {
                self.drain(inner, power, page);
            }
        }
    }

    fn feed(&self, ptr: NonNull<u8>, power: usize) {
        let mut g = self.pump[power].lock();
        // SAFETY: 块已被释放；首 8 字节空闲可写
        unsafe { g.push(ptr) };
    }

    fn suck(&self) {
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
                // SAFETY: 链中块均已被释放、首字为 next 指针
                let next = unsafe { node.cast::<Option<NonNull<u8>>>().read() };
                self.push(node, power);
                this = next;
            }
        }
    }

    fn clear(&self) {
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

struct Pool {
    freepool: Vec<Option<NonNull<u8>>>,
    spare: [Option<usize>; MAX_POWER + 1],
}

impl Pool {
    fn new() -> Self {
        Self {
            freepool: Vec::new(),
            spare: [None; MAX_POWER + 1],
        }
    }

    fn init(&mut self) -> Result<(), InitError> {
        self.freepool
            .try_reserve(MAX_POWER + 1)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freepool.resize_with(MAX_POWER + 1, || None);
        Ok(())
    }
}

static BLOCK_ALLOCATOR: OnceLock<BlockAllocator> = OnceLock::new();

pub(crate) fn heap() -> &'static BlockAllocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

pub(crate) fn flush() {
    for pool in heap().blocks {
        pool.suck();
        pool.clear();
    }
}

pub fn allocator() -> &'static dyn Allocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = BlockAllocator::init()?;
        BLOCK_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing block allocator")
}