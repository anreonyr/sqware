use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use erra::ResultExt;

use crate::core::memory::flat::allocator::{InitError, InitResult, bump, statistics};
use crate::machine::board;
use crate::machine::hart;
use crate::machine::sync::{OnceLock, SpinLock};
use kernel_allocator::Region;
use kernel_allocator::block::{BlockAllocator as Mechanism, Effect};

pub(crate) struct KernelBlock;

// SAFETY: 机器区域来自 board 的实际 free 区间；Bump/Frame 是同一实际 arena 的稳定
// 实例；`release_frame` 只由本分配器拥有、尚未归还的整页触发；统计/禁令直接转发
// Kernel 当前函数。
unsafe impl Effect for KernelBlock {
    fn memory(&self) -> Region {
        let free = board::info().free;
        Region::new(free.base, free.size)
    }

    fn hart_id(&self) -> usize {
        hart::hart_id().get()
    }

    fn hart_count(&self) -> usize {
        hart::hart_count()
    }

    fn install_block_kinds(&self, base: usize, len: usize) -> Result<(), InitError> {
        statistics::install_block_kinds(base, len)
    }

    fn allocate_bump(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        bump::allocator().allocate(layout)
    }

    fn allocate_prime(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        crate::tag!(Prime, crate::core::memory::flat::allocator::frame::allocator().allocate(layout))
    }

    unsafe fn release_frame(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            crate::core::memory::flat::allocator::frame::allocator().deallocate(ptr, layout);
        }
    }

    fn record_block_take(&self, addr: usize, power: usize) {
        statistics::record_block_take(addr, power);
    }

    fn record_block_give(&self, addr: usize, power: usize) {
        statistics::record_block_give(addr, power);
    }

    fn record_pool_take(&self) {
        statistics::record_pool_take();
    }

    fn record_pool_give(&self) {
        statistics::record_pool_give();
    }

    fn assert_allocation_allowed(&self) {
        crate::core::memory::flat::allocator::assert_allocation_allowed();
    }
}

pub(crate) type BlockAllocator = Mechanism<SpinLock<()>, KernelBlock>;

static BLOCK_ALLOCATOR: OnceLock<BlockAllocator> = OnceLock::new();

pub(crate) fn heap() -> &'static BlockAllocator {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

pub(crate) fn flush() {
    heap().flush();
}

pub fn allocator() -> &'static dyn Allocator {
    heap()
}


pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap = BlockAllocator::new(KernelBlock)?;
        BLOCK_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing block allocator")
}
