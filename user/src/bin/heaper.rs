#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use user::env::put;

// heaper：真实走用户堆。每迭代 `Vec` 分配一个非页尺寸块——global allocator 页对齐取整
// → heap envcall → 内核堆窗口位图；`drop` 释放回收。每 2^16 次分配写 'H'（低频心跳，
// 避免高频分配刷屏控制台），验证 `heap_allocate`/`heap_deallocate` 贯通且逐次闭环不泄漏。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let mut n: u64 = 0;
    put("heaper\n").ok();
    loop {
        n = n.wrapping_add(1);

        {
            let _: Vec<u8> = Vec::with_capacity((2048 + 1024) >> 2); // 非页尺寸 → 页对齐取整分配
        }

        if n.is_multiple_of(0xFFFF) {
            put("H\n").ok();
        }
    }
}
