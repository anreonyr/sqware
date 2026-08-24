#![no_std]
#![no_main]
//! counter：每 2^18 次迭代写 'A'，从不让出。

use user::env::put;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 0x3_FFFF == 0 {
            let _ = put("A\n");
        }
    }
}
