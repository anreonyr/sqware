use alloc::alloc::Allocator;
use alloc::boxed::Box;

use erra::ResultExt;

use crate::core::memory::flat::allocator::{InitError, InitResult, bump, statistics};
use crate::machine::sync::SpinLock;
use kernel_allocator::Region;
use kernel_allocator::frame::{Effect, FrameAllocator as Mechanism};
use kernel_allocator::MachineFacts;

pub(crate) struct KernelFrame;

// SAFETY: facts 来自 board 的实际机器事实；boundary/frontier 取自同一个实际 Bump
// 实例；帧池 backing 就是该 Bump 尚未消费的 free DRAM，生存期覆盖 Frame 与派生
// Block/Hybrid。回调直接调用 Kernel 当前的统计/输出/禁令函数。
unsafe impl Effect for KernelFrame {
    fn facts(&self) -> MachineFacts {
        let m = crate::machine::board::info();
        MachineFacts {
            free: Region::new(m.free.base, m.free.size),
            reserved: m
                .reserved
                .map(|region| region.map(|r| Region::new(r.base, r.size))),
        }
    }

    fn bump_boundary(&self) -> usize {
        bump::boundary()
    }

    fn bump_frontier(&self) -> usize {
        bump::frontier()
    }

    fn install_frame_kinds(&self, frames: usize) -> Result<(), InitError> {
        statistics::install_frame_kinds(frames)
    }

    fn record_frame_take(&self, index: usize, power: usize) {
        statistics::record_frame_take(index, power);
    }

    fn record_frame_give(&self, index: usize) {
        statistics::record_frame_give(index);
    }

    fn assert_allocation_allowed(&self) {
        crate::core::memory::flat::allocator::assert_allocation_allowed();
    }

    fn report(&self, args: core::fmt::Arguments<'_>) {
        crate::machine::console::_write(format_args!("{args}\n"));
    }
}

type FrameAllocator = Mechanism<SpinLock<()>, KernelFrame>;

static FRAME_ALLOCATOR: crate::machine::sync::OnceLock<&'static FrameAllocator> =
    crate::machine::sync::OnceLock::new();

/// 最低非空伙伴桶的幂（只读观测；测试夹具用它判断是否已把小于 8 MiB 的块抽干）。
#[cfg(all(debug_assertions, feature = "mega-leaf"))]
pub(crate) fn lowest_free_power() -> Option<usize> {
    FRAME_ALLOCATOR
        .get()
        .and_then(|allocator| allocator.lowest_free_power())
}

pub(crate) fn instance() -> &'static FrameAllocator {
    FRAME_ALLOCATOR
        .get()
        .expect("frame allocator not initialized")
}

pub fn allocator() -> &'static dyn Allocator {
    instance()
}

pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let mut slot =
            Box::<FrameAllocator>::try_new_uninit().map_err(|_| InitError::OutOfMemory)?;
        slot.write(FrameAllocator::new(KernelFrame)?);
        let heap = Box::leak(unsafe { slot.assume_init() });
        FRAME_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing frame allocator")
}
