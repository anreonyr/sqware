use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::{InitError, Lock};
use env::PAGE_SIZE;

mod effect;
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
pub use effect::Effect;
use inner::BlockInner;
use pool::Pool;
use tally::Tally;

const MIN_POWER: usize = 3;
const MAX_POWER: usize = (PAGE_SIZE / 2).ilog2() as usize;
pub(crate) const MAX_SIZE: usize = 1 << MAX_POWER;
const PAGE_SHIFT: usize = PAGE_SIZE.ilog2() as usize;

fn power_of(size: usize) -> usize {
    size.max(1usize << MIN_POWER).next_power_of_two().ilog2() as usize
}

pub struct BlockAllocator<L: Lock + 'static, E: Effect> {
    blocks: &'static [BlockInner<L>],
    tally: &'static Tally<L>,
    effect: E,
}

impl<L: Lock + 'static, E: Effect> BlockAllocator<L, E> {
    pub fn new(effect: E) -> Result<Self, InitError> {
        let nodes = effect.hart_count();
        assert!(nodes > 0, "block init: no harts");
        let m = effect.memory();

        let tally = {
            let meta_len = m.size.div_ceil(PAGE_SIZE);
            let meta = effect
                .allocate_bump(
                    Layout::from_size_align(
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
                Box::try_new(Tally::new(m.base, cells, meta_len))
                    .map_err(|_| InitError::OutOfMemory)?,
            )
        };
        effect.install_block_kinds(m.base, m.size)?;

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
            effect,
        })
    }

    /// 页所属的池下标；越界或未归属返回 `None`。这是池归属读数，不是授权。
    pub fn pool_of(&self, pa: usize) -> Option<usize> {
        self.tally.owner_of(pa)
    }

    /// 每池先抽干泵，再清掉本池的空闲页与 spare 迟滞。
    pub fn flush(&self) {
        for pool in self.blocks {
            pool.suck(&self.effect);
            pool.clear(&self.effect);
        }
    }
}

unsafe impl<L: Lock + 'static, E: Effect> Allocator for BlockAllocator<L, E> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.effect.assert_allocation_allowed();
        let power = power_of(layout.size());
        if power > MAX_POWER || layout.align() > (1usize << power) {
            return Err(AllocError);
        }
        let me = self.effect.hart_id();
        let pool = &self.blocks[me];
        let addr = pool.pull(&self.effect, power).ok_or(AllocError)?;
        self.effect.record_block_take(addr, power);

        Ok(NonNull::slice_from_raw_parts(
            unsafe { NonNull::new_unchecked(addr as *mut u8) },
            layout.size(),
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let power = power_of(layout.size());
        let pa = ptr.addr().get();
        let Some(home) = self.pool_of(pa) else { return };
        self.effect.record_block_give(pa, power);

        let me = self.effect.hart_id();
        let pool = &self.blocks[home];
        if home == me {
            pool.push(&self.effect, ptr, power);
        } else {
            pool.feed(ptr, power);
        }
    }
}
