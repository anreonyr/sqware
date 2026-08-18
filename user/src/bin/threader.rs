#![no_std]
#![no_main]
//! threader：入口参数 arg(a0) 分支行为——arg==0 → 'A' 循环，否则 'B' 循环。
//! 同一空间双线程各自长跑（多核下分布在两核真实并行）。对位旧 blob program_d。

use user::env::put;

#[unsafe(no_mangle)]
extern "C" fn main(arg: usize) -> ! {
    let ch = if arg == 0 { b'A' } else { b'B' };
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 0x3_FFFF == 0 {
            let _ = put(ch);
        }
    }
}
