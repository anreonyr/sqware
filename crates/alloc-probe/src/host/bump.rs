use core::alloc::Allocator;

use kernel_allocator::Region;
use kernel_allocator::bump::BumpAllocator;

use super::Lock;
use crate::fixture;
use crate::shim_lock::SpinLock;

static BUMP_ALLOCATOR: BumpAllocator<Lock> = BumpAllocator::new(SpinLock::new(()));

pub(crate) fn init() {
    let free = fixture::info().free;
    // SAFETY: 台面是 harness 持有到进程结束的独占宿主缓冲；Bump 与派生 Frame
    // 都只从这一实例取前沿，且 harness 的 INIT OnceLock 保证只装一次。
    unsafe { BUMP_ALLOCATOR.init(Region::new(free.base, free.size)) }.expect("bump::init");
}

pub(crate) fn allocator() -> &'static dyn Allocator {
    &BUMP_ALLOCATOR
}

pub(crate) fn boundary() -> usize {
    BUMP_ALLOCATOR.boundary()
}

pub(crate) fn frontier() -> usize {
    BUMP_ALLOCATOR.frontier()
}
