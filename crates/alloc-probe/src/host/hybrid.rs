use core::alloc::Allocator;

use kernel_allocator::hybrid::{Effect, HybridAllocator};

pub(crate) struct HostHybrid;

// SAFETY: block/frame 返回本 crate 的同一对稳定实例，均实现 Sync 并共享 harness
// 台面；宿主分配禁令按既有宿主范围为空。该 effect 只在机制静态里构造一次。
unsafe impl Effect for HostHybrid {
    fn block(&self) -> &'static (dyn Allocator + Sync) {
        super::block::heap()
    }

    fn frame(&self) -> &'static (dyn Allocator + Sync) {
        super::frame::instance()
    }

    fn assert_allocation_allowed(&self) {}
}

static HYBRID_ALLOCATOR: HybridAllocator<HostHybrid> = HybridAllocator::new(HostHybrid);

pub(crate) fn allocator() -> &'static dyn Allocator {
    &HYBRID_ALLOCATOR
}
