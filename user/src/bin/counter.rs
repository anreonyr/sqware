#![no_std]
#![no_main]
//! counter：每 2^18 次迭代写 'A'，从不让出——靠定时器抢占切走。
//! 对位旧 blob program_a。

use core::arch::global_asm;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call main",
    "1: j 1b", // 兜底：main 返回不应发生
);

use user::put;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 0x3_FFFF == 0 {
            put(b'A');
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
