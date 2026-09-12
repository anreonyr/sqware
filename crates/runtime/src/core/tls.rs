//! 用户 TLS 地基 — 每线程独立 tp 指向的 TLS 块。

use env::EnvResult;

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

pub fn alloc() -> EnvResult<usize> {
    memory::allocate(TLS_SIZE)
}

/// 归还本线程的 TLS 块（**任务退场前必调**）。
///
/// 为什么必须有：TLS 块是内核给的**一整页**（`memory::allocate` 按页取整），
/// 内核**不认**它是谁的——`bury` 只归还 `TaskIdent` 上记着的那两个 Span（栈 /
/// trap 帧）。于是每个 spawn 出来的任务都把一页永久留在域空间里：churn
/// 实测就是这条（order-0 帧随任务数线性流失，见 `docs/` 里的泄漏探针记录）。
///
/// 归还走的是**用户态既有原语** `MemoryCall::Deallocate`（内核侧按 `(addr, size)`
/// 精确匹配 `HeapWindow` 的簿记再摘映射），故不需要任何新 ABI。
pub fn free() {
    let tp = base();
    if tp == 0 {
        return; // 未装配（理论上不可达）：不制造第二处失败
    }
    let _ = memory::deallocate(tp, TLS_SIZE);
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
