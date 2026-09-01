#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use user::PAGE_SIZE;
use user::env::{io::put, memory};

// stressor：用户堆分配器压力测试——三幕：多尺寸混合 / 持有-全释放 / 失败路径。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("stressor\n");

    for i in 0..256usize {
        let size = PAGE_SIZE * (1 << (i % 4));
        let va = memory::allocate(size).expect("stressor: alloc 1");
        memory::deallocate(va, size).expect("stressor: free 1");
    }
    let _ = put("1\n");

    let mut held: Vec<(usize, usize)> = Vec::new();
    for s in [PAGE_SIZE, PAGE_SIZE * 4, PAGE_SIZE * 16, PAGE_SIZE] {
        let va = memory::allocate(s).expect("stressor: alloc 2");
        held.push((va, s));
    }
    for (va, s) in held.drain(..) {
        memory::deallocate(va, s).expect("stressor: free 2");
    }
    let _ = put("2\n");

    memory::allocate(usize::MAX / 2).expect_err("stressor: huge alloc must fail");
    let va = memory::allocate(PAGE_SIZE).expect("stressor: re-alloc after oom");
    memory::deallocate(va, PAGE_SIZE).expect("stressor: free 3");
    let _ = put("3\n");

    let _ = put("stressor: ok\n");
}