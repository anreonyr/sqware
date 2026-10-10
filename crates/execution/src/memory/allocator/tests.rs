use super::*;
use crate::memory::{BYTES, CALLS, FAIL};
use std::sync::{Arc, Barrier, atomic::Ordering};

#[test]
fn pooling_alignment_realloc_failure_reclaim_and_cross_task_free() {
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
}
