#![no_std]
#![no_main]

use core::arch::global_asm;

use user::put;

// 入口：内核把 sp 设为栈顶、sepc 指向 _start（= e_entry @ 0x10000），直接 call main。
global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call main",
    "1: j 1b", // 兜底：main 返回不应发生
);

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 对照旧 demo program_c：写 'C' 后退出，验证 parser→loader→TaskBuilder 全链
    put(b'C');
    user::exit()
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
