use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use kernel_allocator::block::{BlockAllocator, Effect};
use kernel_allocator::{InitError, Region};

use super::Lock;
use crate::fixture;
use crate::shim_lock::OnceLock;

pub(crate) struct HostBlock;

// SAFETY: 机器区域来自 harness 的独占台面；Bump/Frame 是同一实际 arena 的稳定
// 实例；`release_frame` 只由本分配器拥有的 pool 页触发；宿主统计走本 crate 的
// 原子计数，分配禁令按既有宿主范围为空。该 effect 只在一次性装台里构造。
unsafe impl Effect for HostBlock {
    fn memory(&self) -> Region {
        fixture::info().free
    }

    fn hart_id(&self) -> usize {
        fixture::hart_id().get()
    }

    fn hart_count(&self) -> usize {
        fixture::hart_count()
    }

    fn install_block_kinds(&self, _base: usize, _len: usize) -> Result<(), InitError> {
        Ok(())
    }

    fn allocate_bump(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::bump::allocator().allocate(layout)
    }

    fn allocate_prime(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::frame::allocator().allocate(layout)
    }

    unsafe fn release_frame(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { super::frame::allocator().deallocate(ptr, layout) };
    }

    fn record_block_take(&self, _addr: usize, _power: usize) {
        super::statistics::BLOCK_TAKE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_block_give(&self, _addr: usize, _power: usize) {
        super::statistics::BLOCK_GIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_pool_take(&self) {
        super::statistics::POOL_TAKE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_pool_give(&self) {
        super::statistics::POOL_GIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn assert_allocation_allowed(&self) {}
}

pub(crate) type Block = BlockAllocator<Lock, HostBlock>;

static BLOCK_ALLOCATOR: OnceLock<Block> = OnceLock::new();

pub(crate) fn init() {
    let heap = Block::new(HostBlock).expect("block::init");
    assert!(BLOCK_ALLOCATOR.set(heap).is_ok(), "block::init once");
}

pub(crate) fn heap() -> &'static Block {
    BLOCK_ALLOCATOR.get().expect("block heap not initialized")
}

pub fn allocator() -> &'static dyn Allocator {
    heap()
}
