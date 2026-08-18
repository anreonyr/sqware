//! 共享入口（user-bin 引导 + panic 处理）。
//!
//! 内核 loader 契约：`sp` 置栈顶、`sepc` 指向 `_start`(= e_entry)，`sret` 进用户态。
//! `_start` 保留 a0（`main` 的入口参数 arg）后 `call main`；`main` 每 bin 各写一份。
//! bins 只需定义 `#[unsafe(no_mangle)] extern "C" fn main(arg: usize) -> !`。

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
