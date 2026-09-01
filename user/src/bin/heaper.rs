#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use user::env::io::put;

// heaper：每迭代分配并释放一个非页尺寸 `Vec`；每 0xF 次写 'H'。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let mut n: u64 = 0;
    let _ = put("heaper\n");
    loop {
        n = n.wrapping_add(1);
        {
            let _: Vec<u8> = Vec::with_capacity((2048 + 1024) >> 2);
        }
        if n.is_multiple_of(0xF) {
            let _ = put("H\n");
        }
    }
}