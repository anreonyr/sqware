use core::alloc::Allocator;

use alloc::boxed::Box;

use kernel_allocator::frame::{Effect, FrameAllocator};
use kernel_allocator::{InitError, MachineFacts};

use super::Lock;
use crate::fixture;
use crate::shim_lock::OnceLock;

pub(crate) struct HostFrame;

// SAFETY: facts 来自 harness 实际配置的独占台面；boundary/frontier 取同一 Bump
// 实例；宿主上类目表与分配禁令按既有宿主范围保持空实现。该 effect 只在 harness
// 的一次性装台里构造，不对外暴露。
unsafe impl Effect for HostFrame {
    fn facts(&self) -> MachineFacts {
        let m = fixture::info();
        MachineFacts {
            free: m.free,
            reserved: m.reserved,
        }
    }

    fn bump_boundary(&self) -> usize {
        super::bump::boundary()
    }

    fn bump_frontier(&self) -> usize {
        super::bump::frontier()
    }

    fn install_frame_kinds(&self, _frames: usize) -> Result<(), InitError> {
        Ok(())
    }

    fn record_frame_take(&self, _index: usize, _power: usize) {
        super::statistics::FRAME_TAKE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_frame_give(&self, _index: usize) {
        super::statistics::FRAME_GIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn assert_allocation_allowed(&self) {}

    fn report(&self, args: core::fmt::Arguments<'_>) {
        println!("{args}");
    }
}

pub(crate) type Frame = FrameAllocator<Lock, HostFrame>;

static FRAME_ALLOCATOR: OnceLock<&'static Frame> = OnceLock::new();

pub(crate) fn init() {
    let mut slot = Box::<Frame>::try_new_uninit().expect("frame slot");
    slot.write(Frame::new(HostFrame).expect("frame::init"));
    let heap = Box::leak(unsafe { slot.assume_init() });
    assert!(FRAME_ALLOCATOR.set(heap).is_ok(), "frame::init once");
}

pub(crate) fn instance() -> &'static Frame {
    FRAME_ALLOCATOR.get().expect("frame allocator not initialized")
}

pub fn allocator() -> &'static dyn Allocator {
    instance()
}
