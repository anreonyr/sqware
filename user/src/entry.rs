//! 共享入口（user-bin 引导 + panic 处理）。

use core::arch::global_asm;

use crate::env::{io::put, room::exit};

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call tls_bootstrap",
    "    call main",
    "    call exit_trampoline", // main 返回（理论上 !，兜底）→ room::exit
    "1: j 1b", // ec 返回则兜底循环
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { crate::core::tls::bootstrap() }
}

#[unsafe(no_mangle)]
extern "C" fn exit_trampoline() -> ! {
    exit()
}

// panic_handler 路径禁忌：不能走 writeln!/format!（潜在分配 → 双重 panic）。
// 走直接 put 字符串 + 整数的十进制逐位写。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = put("user paniced\n");
    if let Some(loc) = info.location() {
        let _ = put("  at ");
        let _ = put(loc.file());
        let _ = put(":");
        let mut n = loc.line();
        if n == 0 {
            let _ = put("0");
        } else {
            let mut buf = [0u8; 20];
            let mut i = 20;
            while n > 0 && i > 0 {
                i -= 1;
                buf[i] = b'0' + (n % 10) as u8;
                n /= 10;
            }
            let _ = put(core::str::from_utf8(&buf[i..]).unwrap_or("?"));
        }
        let _ = put("\n");
    }
    exit()
}

