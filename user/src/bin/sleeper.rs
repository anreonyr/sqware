#![no_std]
#![no_main]
//! sleeper：写 'E' 后睡眠 16 量子（任务级阻塞：Running → Blocked → unpark 唤醒），
//! 循环。对位旧 blob program_e。

use core::arch::global_asm;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call main",
    "1: j 1b", // 兜底：main 返回不应发生
);

use user::{put, sleep};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    loop {
        put(b'E');
        sleep(16);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
