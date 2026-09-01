#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use user::env::put;

// heaper：每迭代分配并释放一个非页尺寸 `Vec`；每 0xF 次写 'H'（低频心跳），
// 验证用户堆贯通且逐次闭环不泄漏。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let mut n: u64 = 0;
    put("heaper\n").ok();
    loop {
        n = n.wrapping_add(1);

        {
            let _: Vec<u8> = Vec::with_capacity((2048 + 1024) >> 2); // 非页尺寸 → 页对齐取整分配
        }

        if n.is_multiple_of(0xF) {
            put("H\n").ok();
        }
    }
}
