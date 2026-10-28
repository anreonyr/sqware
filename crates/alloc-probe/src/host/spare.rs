use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;

use kernel_allocator::spare::{Effect, SpareAllocator};

use super::Lock;
use crate::fixture;
use crate::shim_lock::OnceLock;

pub(crate) struct HostSpare;

// SAFETY: trace 载荷按既有宿主常量；仓区从同一实际 Hybrid 实例取得并在 Spare
// 生存期内独占；统计走本 crate 的原子计数。该 effect 只在一次性装台里构造。
unsafe impl Effect for HostSpare {
    fn trace_payload_bytes(&self) -> usize {
        4096 * fixture::hart_count().max(1)
    }

    fn allocate_spare(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::hybrid::allocator().allocate(layout)
    }

    fn record_spare_take(&self, _bytes: usize) {
        super::statistics::SPARE_TAKE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_spare_give(&self, _bytes: usize) {
        super::statistics::SPARE_GIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn assert_allocation_allowed(&self) {}
}

pub(crate) type Spare = SpareAllocator<Lock, HostSpare>;

static SPARE_ALLOCATOR: OnceLock<&'static Spare> = OnceLock::new();

pub(crate) fn init() {
    let heap = Box::leak(
        Box::try_new(Spare::new(HostSpare).expect("spare::init")).expect("spare slot"),
    );
    assert!(SPARE_ALLOCATOR.set(heap).is_ok(), "spare::init once");
}

pub(crate) fn spare() -> &'static Spare {
    SPARE_ALLOCATOR.get().expect("spare allocator not initialized")
}

pub(crate) fn allocator() -> &'static dyn Allocator {
    spare()
}
