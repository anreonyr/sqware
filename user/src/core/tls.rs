//! 用户 TLS 地基 — 每线程独立 tp 指向的 TLS 块。

use ubi::UResult;

use crate::PAGE_SIZE;
use crate::env::memory;

// 硬不变量：base() 仅在装配点后有效；bootstrap 仅在主线程出生点调用恰好一次。

pub const TLS_SIZE: usize = PAGE_SIZE;

#[inline]
pub fn base() -> usize {
    let tp: usize;
    // SAFETY: 纯读 tp，无副作用。
    unsafe {
        core::arch::asm!("mv {0}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
    }
    tp
}

pub fn alloc() -> UResult<usize> {
    memory::allocate(TLS_SIZE)
}

/// # Safety
/// 仅在主线程出生点（`_start` → `main` 之间）调用恰好一次。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bootstrap() {
    let addr = alloc().expect("tls bootstrap alloc failed");
    // SAFETY: 写 tp（用户态自由；本线程刚出生，无旧值）。
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) addr, options(nomem, nostack, preserves_flags));
    }
}
