#![cfg(debug_assertions)]

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::hybrid;

const SIZES: [usize; 7] = [16, 64, 128, 256, 512, 1024, 2048];
const STEPS: usize = 64;
const HELD: usize = 8;
const FRAME_SIZES: [usize; 4] = [4096, 8192, 16384, 32768];
const FRAME_STEPS: usize = 64;
const FRAME_HELD: usize = 8;

pub fn accept() {
    let a = hybrid::allocator();
    let kinds_before = crate::memory::allocator::statistics::kinds();

    for i in 0..STEPS {
        let l = Layout::from_size_align(SIZES[i % SIZES.len()], 8).unwrap();
        // SAFETY: 分配块当轮即释（闭环）
        unsafe {
            let b = a.allocate(l).expect("stress: alloc 1");
            a.deallocate(b.cast(), l);
        }
    }

    let mut held: Vec<(NonNull<[u8]>, Layout)> = Vec::new();
    for i in 0..HELD {
        let l = Layout::from_size_align(PAGE_SIZE / 16 * (i + 1), 8).unwrap();
        let b = a.allocate(l).expect("stress: alloc 2");
        held.push((b, l));
    }
    for (b, l) in held.drain(..) {
        // SAFETY: b 来自本幕 allocate、layout 同源
        unsafe { a.deallocate(b.cast(), l) };
    }
    let l = Layout::from_size_align(1024, 8).unwrap();
    // SAFETY: 复验块当轮即释
    unsafe {
        let b = a.allocate(l).expect("stress: re-alloc after merge");
        a.deallocate(b.cast(), l);
    }

    for i in 0..FRAME_STEPS {
        let size = FRAME_SIZES[i % FRAME_SIZES.len()];
        let l = Layout::from_size_align(size, PAGE_SIZE).unwrap();
        // SAFETY: 块当轮即释（闭环）
        unsafe {
            let b = a.allocate(l).expect("stress: frame alloc");
            a.deallocate(b.cast(), l);
        }
    }

    let mut fheld: Vec<(NonNull<[u8]>, Layout)> = Vec::new();
    for i in 0..FRAME_HELD {
        let l = Layout::from_size_align(PAGE_SIZE * (1 << (i % 4)), PAGE_SIZE).unwrap();
        let b = a.allocate(l).expect("stress: frame hold alloc");
        fheld.push((b, l));
    }
    for (b, l) in fheld.drain(..) {
        // SAFETY: b 来自本幕 allocate、layout 同源
        unsafe { a.deallocate(b.cast(), l) };
    }

    let mut drained: Vec<(NonNull<[u8]>, Layout)> = Vec::new();
    let mut n = 0usize;
    loop {
        let l = Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap();
        match a.allocate(l) {
            Ok(b) => {
                drained.push((b, l));
                n += 1;
            }
            Err(_) => break,
        }
    }
    crate::expect!(n > 0, "frame drain: no blocks ever allocated");
    let l = Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap();
    crate::expect!(
        a.allocate(l).is_err(),
        "frame drain: alloc after exhaustion must fail"
    );
    for (b, l) in drained.drain(..) {
        // SAFETY: b 来自本幕 allocate、layout 同源
        unsafe { a.deallocate(b.cast(), l) };
    }
    let l = Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap();
    // SAFETY: 复验块当轮即释
    unsafe {
        let b = a.allocate(l).expect("stress: frame after drain");
        a.deallocate(b.cast(), l);
    }

    drop(drained);
    drop(held);
    drop(fheld);

    let kinds_after = crate::memory::allocator::statistics::kinds();
    for (k, n) in kinds_after.nonzero() {
        crate::expect!(
            n == kinds_before.get(k),
            "frame kind {}: {} → {}（逐类净额应为 0）",
            k.name(),
            kinds_before.get(k),
            n
        );
    }
}
