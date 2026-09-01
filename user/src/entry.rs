//! 共享入口（user-bin 引导 + panic 处理）。

use core::arch::global_asm;
use core::fmt::{self, Write};

use crate::env::{io::put, room::exit};

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call tls_bootstrap", // 主线程 TLS：tp → 本线程块（装配点）
    "    call main",
    "    call exit_trampoline", // main 返回（理论上 !，兜底） → room::exit
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

/// 把 `put` 包成 `fmt::Write`—— panic_handler 唯一消费者。
struct PanicWriter;

impl Write for PanicWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = put(s);
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut w = PanicWriter;
    if let Some(loc) = info.location() {
        let _ = writeln!(w, "panicked at {loc}");
    } else {
        let _ = writeln!(w, "panicked at <unknown location>");
    }
    let _ = writeln!(w, "{}", info.message());
    exit()
}