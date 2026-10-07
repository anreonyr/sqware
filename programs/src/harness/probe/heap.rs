use alloc::{alloc::{alloc, alloc_zeroed, dealloc, realloc}, vec::Vec};
use core::alloc::Layout;

pub fn acceptance() {
    pooling();
    layouts();
    concurrent();
    protocol::debug::put("heap: pooled pages, aligned zeroing, realloc and cross-task release passed");
}

fn pooling() {
    let layout = Layout::from_size_align(64, 8).unwrap();
    let mut pointers = [0usize; 128];
    let mut pages = 0;
    for index in 0..pointers.len() {
        // SAFETY: each live allocation is checked and freed with its original layout.
        let ptr = unsafe { alloc(layout) };
        assert!(!ptr.is_null());
        let page = ptr as usize / env::PAGE_SIZE;
        if !pointers[..index].iter().any(|addr| addr / env::PAGE_SIZE == page) {
            pages += 1;
        }
        unsafe { ptr.write_bytes(index as u8, 64) };
        pointers[index] = ptr as usize;
    }
    assert!(pages < 16, "heap: small objects still consume separate pages");
    for (index, addr) in pointers.into_iter().enumerate() {
        let ptr = addr as *mut u8;
        assert!(unsafe { core::slice::from_raw_parts(ptr, 64) }.iter().all(|&b| b == index as u8));
        unsafe { dealloc(ptr, layout) };
    }
}

fn layouts() {
    for align in [8, 64, 4096, 16384] {
        let layout = Layout::from_size_align(257, align).unwrap();
        // SAFETY: read and write only live allocations within their current layouts.
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % align, 0);
        assert!(unsafe { core::slice::from_raw_parts(ptr, 257) }.iter().all(|&b| b == 0));
        unsafe { ptr.write_bytes(53, 257) };
        let grown = unsafe { realloc(ptr, layout, 32768) };
        assert!(!grown.is_null());
        assert_eq!(grown as usize % align, 0);
        assert!(unsafe { core::slice::from_raw_parts(grown, 257) }.iter().all(|&b| b == 53));
        let shrunk = unsafe { realloc(grown, Layout::from_size_align(32768, align).unwrap(), 31) };
        assert!(!shrunk.is_null());
        assert_eq!(shrunk as usize % align, 0);
        assert!(unsafe { core::slice::from_raw_parts(shrunk, 31) }.iter().all(|&b| b == 53));
        unsafe { dealloc(shrunk, Layout::from_size_align(31, align).unwrap()) };
    }
}

fn concurrent() {
    let mut workers = Vec::new();
    for byte in 0..4u8 {
        let input = alloc::vec![byte; 257];
        workers.push(execution::unit::join::closure(move || {
            assert!(input.iter().all(|&b| b == byte));
            drop(input);
            for size in [8, 64, 257, 2048, 4097] {
                for _ in 0..128 {
                    let mut bytes = alloc::vec![byte; size];
                    bytes.resize(size * 2, byte);
                    assert!(bytes.iter().all(|&b| b == byte));
                }
            }
            alloc::vec![byte; 257]
        }));
    }
    for (byte, worker) in workers.into_iter().enumerate() {
        let output = worker.join();
        assert!(output.iter().all(|&b| b == byte as u8));
        drop(output);
    }
}
