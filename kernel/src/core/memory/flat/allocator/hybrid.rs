use core::alloc::Allocator;

use crate::core::memory::flat::allocator::{InitResult, block, frame};
use kernel_allocator::hybrid::{Effect, HybridAllocator as Mechanism};

pub(crate) struct KernelHybrid;

// SAFETY: block/frame 返回 Kernel 的实际稳定实例；两者都实现 Sync 并由 Kernel 的
// arena 支撑；禁令直接转发 Kernel 当前函数。
unsafe impl Effect for KernelHybrid {
    fn block(&self) -> &'static (dyn Allocator + Sync) {
        block::heap()
    }

    fn frame(&self) -> &'static (dyn Allocator + Sync) {
        frame::instance()
    }

    fn assert_allocation_allowed(&self) {
        crate::core::memory::flat::allocator::assert_allocation_allowed();
    }
}

pub(crate) static HYBRID_ALLOCATOR: Mechanism<KernelHybrid> = Mechanism::new(KernelHybrid);

pub fn allocator() -> &'static dyn Allocator {
    &HYBRID_ALLOCATOR
}

pub fn init() -> InitResult<()> {
    block::init()?;
    frame::init()?;
    Ok(())
}
