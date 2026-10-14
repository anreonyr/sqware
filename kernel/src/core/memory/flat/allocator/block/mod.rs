use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::vec::Vec;
use erra::ResultExt;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::flat::allocator::{InitError, InitResult, bump};
use crate::machine::board;
use crate::machine::hart;
use crate::machine::sync::OnceLock;

#[path = "cell.rs"]
mod cell;
#[path = "inner.rs"]
mod inner;
#[path = "pool.rs"]
mod pool;
#[path = "pump.rs"]
mod pump;
#[path = "tally.rs"]
mod tally;

use cell::Cell;
use inner::BlockInner;
use pool::Pool;
use tally::Tally;

const MIN_POWER: usize = 3;
const MAX_POWER: usize = (PAGE_SIZE / 2).ilog2() as usize;
pub(super) const MAX_SIZE: usize = 1 << MAX_POWER;
const PAGE_SHIFT: usize = PAGE_SIZE.ilog2() as usize;

fn power_of(size: usize) -> usize {
    size.max(1usize << MIN_POWER).next_power_of_two().ilog2() as usize
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
        let m = board::info();

        let tally = {
            let meta_len = m.free.size.div_ceil(PAGE_SIZE);
            let meta = bump::allocator()
                .allocate(
                    core::alloc::Layout::from_size_align(
                        meta_len * core::mem::size_of::<Cell>(),
                        core::mem::align_of::<Cell>(),
                    )
                    .unwrap(),
                )
                .map_err(|_| InitError::OutOfMemory)?;
            let cells = meta.as_ptr() as *mut Cell;
            unsafe {
                for i in 0..meta_len {
                    cells.add(i).write(Cell::free());
                }
            }
            Box::leak(
                Box::try_new(Tally::new(m.free.base, cells, meta_len))
                    .map_err(|_| InitError::OutOfMemory)?,
            )
        };
        super::statistics::install_block_kinds(m.free.base, m.free.size)?;

        let mut pools = Vec::new();
        pools
            .try_reserve_exact(nodes)
            .map_err(|_| InitError::OutOfMemory)?;
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
        super::assert_allocation_allowed();
        let power = power_of(layout.size());
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
        let power = power_of(layout.size());
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
