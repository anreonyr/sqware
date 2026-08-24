//! 共享入口（user-bin 引导 + panic 处理）。
//!
//! `_start` 保留 a0（`main` 的入口参数）后 `call main`；bin 只需写 `main`。

use core::arch::global_asm;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call main",
    "1: j 1b", // 兜底：main 返回不应发生
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
