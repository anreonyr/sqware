#![no_std]
#![no_main]
//! threader：入口参数 arg(a0) 分支行为——arg==0 → 'A' 循环，否则 'B' 循环。

use user::env::put;

#[unsafe(no_mangle)]
extern "C" fn main(arg: usize) -> ! {
    let out = if arg == 0 { "A\n" } else { "B\n" };
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 0x3_FFFF == 0 {
            let _ = put(out);
        }
    }
}
