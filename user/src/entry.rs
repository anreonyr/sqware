//! 共享入口（user-bin 引导 + panic 处理）。

use core::arch::global_asm;

use crate::env::{io::put, room::exit};

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call tls_bootstrap",
    "    call main",
    "1: j 1b",
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { crate::core::tls::bootstrap() }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = put("user paniced\n");
    exit()
}