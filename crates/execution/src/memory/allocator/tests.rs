use super::*;
use crate::memory::{BYTES, CALLS, FAIL};
use std::sync::{Arc, Barrier, atomic::Ordering};

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn pooling_alignment_realloc_failure_reclaim_and_cross_task_free() {
    let _test = TEST_LOCK.lock().unwrap();
    let references = memory::references();
    let heap = Arc::new(Heap::new(GlobalAllocSource::with_block_size(
        Pages, BLOCK_SIZE,
    )));
    let small = Layout::from_size_align(64, 8).unwrap();
    let calls = CALLS.load(Ordering::Relaxed);
    let mut pointers = Vec::new();
    for byte in 0..256 {
        // SAFETY: each pointer is checked and used within its layout until freed.
        let ptr = unsafe { heap.alloc(small) };
        assert!(!ptr.is_null());
        unsafe { ptr.write_bytes(byte as u8, small.size()) };
        pointers.push(ptr);
    }
    assert!(
        CALLS.load(Ordering::Relaxed) - calls < 8,
        "small objects did not share regions"
    );
    for (byte, ptr) in pointers.into_iter().enumerate() {
        assert!(
            unsafe { std::slice::from_raw_parts(ptr, 64) }
                .iter()
                .all(|&b| b == byte as u8)
        );
        unsafe { heap.dealloc(ptr, small) };
    }
    let retained = BYTES.load(Ordering::Relaxed);
    assert_eq!(
        retained, BLOCK_SIZE,
        "only the metadata region should remain"
    );
    let calls = CALLS.load(Ordering::Relaxed);
    for _ in 0..1024 {
        let ptr = unsafe { heap.alloc(small) };
        assert!(!ptr.is_null());
        unsafe { heap.dealloc(ptr, small) };
    }
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        calls,
        "warm allocations requested pages"
    );

    for align in [1, 8, 64, 4096, 16384, 65536] {
        let layout = Layout::from_size_align(257, align).unwrap();
        let ptr = unsafe { heap.alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % align, 0);
        assert!(
            unsafe { std::slice::from_raw_parts(ptr, 257) }
                .iter()
                .all(|&b| b == 0)
        );
        unsafe { heap.dealloc(ptr, layout) };
    }
    assert_eq!(BYTES.load(Ordering::Relaxed), retained);

    let ptr = unsafe { heap.alloc(small) };
    assert!(!ptr.is_null());
    unsafe { ptr.write_bytes(37, 64) };
    let grown = unsafe { heap.realloc(ptr, small, 128) };
    assert_eq!(ptr, grown, "unobstructed growth should stay in place");
    let grown_layout = Layout::from_size_align(128, 8).unwrap();
    let shrunk = unsafe { heap.realloc(grown, grown_layout, 16) };
    assert_eq!(grown, shrunk);
    assert!(
        unsafe { std::slice::from_raw_parts(shrunk, 16) }
            .iter()
            .all(|&b| b == 37)
    );
    unsafe { heap.dealloc(shrunk, Layout::from_size_align(16, 8).unwrap()) };

    let ptr = unsafe { heap.alloc(small) };
    assert!(!ptr.is_null());
    unsafe { ptr.write_bytes(91, 64) };
    let blocker = unsafe { heap.alloc(small) };
    assert!(!blocker.is_null());
    FAIL.store(true, Ordering::Relaxed);
    assert!(unsafe { heap.realloc(ptr, small, 4 * 1024 * 1024) }.is_null());
    assert!(
        unsafe { std::slice::from_raw_parts(ptr, 64) }
            .iter()
            .all(|&b| b == 91)
    );
    FAIL.store(false, Ordering::Relaxed);
    let moved = unsafe { heap.realloc(ptr, small, 128 * 1024) };
    assert!(!moved.is_null());
    assert_ne!(ptr, moved);
    assert!(
        unsafe { std::slice::from_raw_parts(moved, 64) }
            .iter()
            .all(|&b| b == 91)
    );
    unsafe {
        heap.dealloc(moved, Layout::from_size_align(128 * 1024, 8).unwrap());
        heap.dealloc(blocker, small);
    }
    assert_eq!(
        BYTES.load(Ordering::Relaxed),
        retained,
        "empty region was not returned"
    );

    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|byte| {
            let heap = heap.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..512 {
                    let ptr = unsafe { heap.alloc(small) };
                    assert!(!ptr.is_null());
                    unsafe { ptr.write_bytes(byte, 64) };
                    assert!(
                        unsafe { std::slice::from_raw_parts(ptr, 64) }
                            .iter()
                            .all(|&b| b == byte)
                    );
                    unsafe { heap.dealloc(ptr, small) };
                }
                let ptr = unsafe { heap.alloc(small) };
                assert!(!ptr.is_null());
                unsafe { ptr.write_bytes(byte, 64) };
                (ptr as usize, byte)
            })
        })
        .collect();
    for worker in workers {
        let (addr, byte) = worker.join().unwrap();
        let ptr = addr as *mut u8;
        assert!(
            unsafe { std::slice::from_raw_parts(ptr, 64) }
                .iter()
                .all(|&b| b == byte)
        );
        unsafe { heap.dealloc(ptr, small) };
    }
    drop(heap);
    assert_eq!(
        BYTES.load(Ordering::Relaxed),
        0,
        "dropping a heap leaked its regions"
    );
    assert_eq!(memory::references(), references);
}

#[test]
fn full_table_does_not_allocate_and_inflight_close_keeps_its_slot() {
    let _test = TEST_LOCK.lock().unwrap();
    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let calls = CALLS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);
    let claimed: Vec<_> = (0..SLOTS).map(|_| claim().unwrap()).collect();
    assert!(unsafe { Pages.alloc(layout) }.is_null());
    assert_eq!(CALLS.load(Ordering::Relaxed), calls);
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes);
    for index in claimed {
        abandon(index);
    }

    let ptr = unsafe { Pages.alloc(layout) };
    assert!(!ptr.is_null());
    let (index, view, pending, span) = claim_close(ptr as usize).unwrap();
    assert!(matches!(REGIONS.lock()[index], Slot::Closing { .. }));
    let detached = memory::DETACHED.load(Ordering::Relaxed);
    sweep();
    finish_if_done(index);
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached);
    assert!(matches!(REGIONS.lock()[index], Slot::Closing { .. }));
    let other = claim().unwrap();
    assert_ne!(other, index, "in-flight close released its slot");
    abandon(other);

    settle_mapping(index, view, pending, span, ptr as usize);
    assert!(matches!(REGIONS.lock()[index], Slot::Free));
    assert_eq!(OWED.load(Ordering::Acquire), 0);
    assert_eq!(PENDING.load(Ordering::Acquire), 0);
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes);
}

#[test]
fn failed_close_retries_mapping_and_preserves_the_owners_reference() {
    let _test = TEST_LOCK.lock().unwrap();
    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let bytes = BYTES.load(Ordering::Relaxed);
    memory::set_task(1);
    memory::RELEASE_FAIL.store(true, Ordering::Relaxed);
    let ptr = unsafe { Pages.alloc(layout) };
    assert!(!ptr.is_null());
    let index = REGIONS
        .lock()
        .iter()
        .position(|slot| matches!(slot, Slot::Live { address, .. } if *address == ptr as usize))
        .unwrap();
    assert_eq!(PENDING.load(Ordering::Acquire), 1);
    let released = memory::RELEASED.load(Ordering::Relaxed);
    let detached = memory::DETACHED.load(Ordering::Relaxed);

    memory::set_task(2);
    memory::DETACH_FAIL.store(true, Ordering::Relaxed);
    unsafe { Pages.dealloc(ptr, layout) };
    assert!(matches!(REGIONS.lock()[index], Slot::Owed { .. }));
    assert_eq!(OWED.load(Ordering::Acquire), 1);
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes + PAGE_SIZE);
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached + 1);
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released);

    memory::DETACH_FAIL.store(false, Ordering::Relaxed);
    sweep();
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached + 2);
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released);
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes);
    assert_eq!(OWED.load(Ordering::Acquire), 0);
    assert!(matches!(
        REGIONS.lock()[index],
        Slot::Done { pending: Some((owner, _)) } if owner == TaskId::new(1)
    ));
    finish_if_done(index);
    assert!(matches!(REGIONS.lock()[index], Slot::Done { .. }));

    memory::set_task(1);
    sweep();
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released + 1);
    assert!(matches!(REGIONS.lock()[index], Slot::Done { .. }));
    memory::RELEASE_FAIL.store(false, Ordering::Relaxed);
    sweep();
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released + 2);
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached + 2);
    assert!(matches!(REGIONS.lock()[index], Slot::Free));
    assert_eq!(PENDING.load(Ordering::Acquire), 0);
    assert_eq!(memory::references(), 0);
    sweep();
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released + 2);
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached + 2);
}

#[test]
fn live_pending_reference_retries_without_closing_the_mapping() {
    let _test = TEST_LOCK.lock().unwrap();
    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let bytes = BYTES.load(Ordering::Relaxed);
    memory::set_task(1);
    memory::RELEASE_FAIL.store(true, Ordering::Relaxed);
    let ptr = unsafe { Pages.alloc(layout) };
    assert!(!ptr.is_null());
    let index = REGIONS
        .lock()
        .iter()
        .position(|slot| matches!(slot, Slot::Live { address, .. } if *address == ptr as usize))
        .unwrap();
    let released = memory::RELEASED.load(Ordering::Relaxed);
    let detached = memory::DETACHED.load(Ordering::Relaxed);
    assert_eq!(OWED.load(Ordering::Acquire), 0);
    assert_eq!(PENDING.load(Ordering::Acquire), 1);
    memory::RELEASE_FAIL.store(false, Ordering::Relaxed);
    sweep();
    assert!(matches!(
        REGIONS.lock()[index],
        Slot::Live { pending: None, .. }
    ));
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released + 1);
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached);
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes + PAGE_SIZE);
    assert_eq!(PENDING.load(Ordering::Acquire), 0);
    unsafe { Pages.dealloc(ptr, layout) };
    assert!(matches!(REGIONS.lock()[index], Slot::Free));
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes);
}

#[test]
fn stale_cleanup_snapshots_cannot_close_new_views_or_restore_released_references() {
    let _test = TEST_LOCK.lock().unwrap();
    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let bytes = BYTES.load(Ordering::Relaxed);
    memory::set_task(1);
    let ptr = unsafe { Pages.alloc(layout) };
    assert!(!ptr.is_null());
    let (index, view, pending, span) = claim_close(ptr as usize).unwrap();
    assert!(pending.is_none());
    mark_owed(index, ptr as usize, span, view);
    sweep();
    assert!(matches!(REGIONS.lock()[index], Slot::Free));

    // A paused scan observed the old Owed entry; another scan settled and reused it.
    let replacement = unsafe { Pages.alloc(layout) };
    assert!(!replacement.is_null());
    assert!(matches!(
        REGIONS.lock()[index],
        Slot::Live { address, .. } if address == replacement as usize
    ));
    let detached = memory::DETACHED.load(Ordering::Relaxed);
    assert!(claim_owed(index).is_none());
    assert_eq!(memory::DETACHED.load(Ordering::Relaxed), detached);
    unsafe { Pages.dealloc(replacement, layout) };

    memory::RELEASE_FAIL.store(true, Ordering::Relaxed);
    let ptr = unsafe { Pages.alloc(layout) };
    assert!(!ptr.is_null());
    let (index, view, pending, span) = claim_close(ptr as usize).unwrap();
    memory::RELEASE_FAIL.store(false, Ordering::Relaxed);
    assert!(release_pending(index, pending));
    assert_eq!(PENDING.load(Ordering::Acquire), 0);
    let released = memory::RELEASED.load(Ordering::Relaxed);

    // The mapping attempt returns after the owner settled the saved reference.
    memory::DETACH_FAIL.store(true, Ordering::Relaxed);
    settle_mapping(index, view, pending, span, ptr as usize);
    assert!(matches!(
        REGIONS.lock()[index],
        Slot::Owed { pending: None, .. }
    ));
    assert_eq!(memory::RELEASED.load(Ordering::Relaxed), released);
    memory::DETACH_FAIL.store(false, Ordering::Relaxed);
    sweep();
    assert!(matches!(REGIONS.lock()[index], Slot::Free));
    assert_eq!(OWED.load(Ordering::Acquire), 0);
    assert_eq!(PENDING.load(Ordering::Acquire), 0);
    assert_eq!(memory::references(), 0);
    assert_eq!(BYTES.load(Ordering::Relaxed), bytes);
}
