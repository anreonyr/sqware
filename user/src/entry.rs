//! 共享入口（user-bin 引导 + panic 处理）。
//!
//! `_start` 先装配主线程 TLS（`tls::bootstrap`：tp → 本线程 TLS 块），保留 a0
//! （`main` 的入口参数）后 `call main`；bin 只需写 `main`。

use core::arch::global_asm;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call tls_bootstrap", // 主线程 TLS：tp → 本线程块（装配点）
    "    call main",
    "1: j 1b", // 兜底：main 返回不应发生
);

/// `_start` 引用的主线程 TLS 装配（经 tls::bootstrap 把 tp 指向本线程块）。
#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    crate::tls::bootstrap()
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
