#![feature(allocator_api)]

//! Owned regression: Frame must build on the **post-install** Bump frontier.
//!
//! `install_frame_kinds` is allowed to consume Bump memory for Kernel metadata. The frame
//! pool must therefore re-read the live frontier after that callback; a frozen pre-install
//! frontier would hand out the classified pages again.
//!
//! The test also statically demonstrates the lock API boundary: `Lock` has no cell/data
//! parameter, and `Locked` is private to the mechanism crate. Safe consumer code therefore
//! has no way to attach two different locks to one `UnsafeCell` and obtain two mutable
//! guards; that is a signature-level fact, not a UB execution test.

use core::alloc::{Allocator, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use kernel_allocator::PAGE_SIZE;
use kernel_allocator::bump::BumpAllocator;
use kernel_allocator::frame::FrameAllocator;
use kernel_allocator::{Lock, Region};

#[path = "support/lock.rs"]
mod lock;
#[path = "support/fixture.rs"]
mod fixture;

use fixture::Fixture;
use lock::TestLock;

#[test]
fn frame_uses_post_install_frontier_and_never_returns_classified_or_reserved_bytes() {
    const PAGES: usize = 128;
    let size = PAGES * PAGE_SIZE;
    let arena = vec![0u8; size + PAGE_SIZE];
    let base = (arena.as_ptr() as usize).next_multiple_of(PAGE_SIZE);

    let bump = BumpAllocator::new(TestLock::new(None));
    // SAFETY: `arena` owns [base, base + size) for the whole test and no other allocator
    // takes this region; base is page aligned and size is page aligned.
    unsafe { bump.init(Region::new(base, size)) }.expect("bump init");

    let reserved_page = Region::new(base + 4 * PAGE_SIZE, PAGE_SIZE);
    let install_addr = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture {
        bump: &bump,
        reserved: reserved_page,
        install_addr: Arc::clone(&install_addr),
    };
    let before = bump.frontier();
    let frame: FrameAllocator<TestLock, Fixture<'_>> =
        FrameAllocator::new(fixture).expect("frame init");
    let after = bump.frontier();
    let install = install_addr.load(Ordering::Relaxed);

    assert!(
        install >= before && install < after,
        "install callback must consume live Bump memory: install={install:#x} before={before:#x} after={after:#x}"
    );

    let expected_base = after.next_multiple_of(PAGE_SIZE);
    assert!(
        expected_base > install,
        "frame base must be at/after post-install frontier"
    );

    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let mut frames = Vec::new();
    while let Ok(buf) = frame.allocate(layout) {
        let addr = buf.cast::<u8>().as_ptr() as usize;
        assert!(
            addr >= expected_base,
            "frame returned pre-boundary memory: {addr:#x} < {expected_base:#x}"
        );
        assert!(
            addr != install,
            "frame returned the page consumed by install_frame_kinds: {addr:#x}"
        );
        assert!(
            !(addr >= reserved_page.base && addr < reserved_page.base + reserved_page.size),
            "frame returned reserved hole: {addr:#x}"
        );
        frames.push(addr);
    }

    assert!(!frames.is_empty(), "fixture must yield usable frames");
    frames.sort_unstable();
    for pair in frames.windows(2) {
        assert!(
            pair[0] + PAGE_SIZE <= pair[1],
            "frame delivery overlaps: {:#x} and {:#x}",
            pair[0],
            pair[1]
        );
    }
}
