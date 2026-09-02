#![no_std]
#![no_main]
//! yielder：每迭代主动让出（用户面 starve），每 2^16 次让出写 'B'。

use user::env::{io::put, room::starve};

#[unsafe(no_mangle)]
extern "C" fn main() {
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 0xFFFF == 0 {
            let _ = put("B\n");
        }
        let _ = starve();
    }
}
