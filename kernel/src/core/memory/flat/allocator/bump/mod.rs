use alloc::alloc::Allocator;

use erra::ResultExt;

use crate::core::memory::flat::allocator::InitResult;
use crate::machine::sync::SpinLock;
use kernel_allocator::Region;
use kernel_allocator::bump::BumpAllocator;

pub(crate) static BUMP_ALLOCATOR: BumpAllocator<SpinLock<()>> =
    BumpAllocator::new(SpinLock::new(()));

pub fn allocator() -> &'static dyn Allocator {
    &BUMP_ALLOCATOR
}

pub(crate) fn boundary() -> usize {
    BUMP_ALLOCATOR.boundary()
}

pub(crate) fn frontier() -> usize {
    BUMP_ALLOCATOR.frontier()
}

pub fn init() -> InitResult<()> {
    let free = crate::machine::board::info().free;
    // SAFETY: board::info().free 是内核实际拥有的 free DRAM 区间；它在整个分配器
    // 生存期内独占有效，Bump 与随后派生的 Frame 都只从这一实例取前沿。
    unsafe { BUMP_ALLOCATOR.init(Region::new(free.base, free.size)) }
        .annotate("initializing bump allocator")
}
