//! 用户 TLS 地基 — 每线程独立 tp 指向的 TLS 块。
//!
//! 机制层（非标准 `thread_local!`）：RISC-V psABI 中 tp（x4）的规范用途是
//! **线程指针**（TLS 块基址）。本模块把它接到用户线程模型上：
//!
//! - [`alloc`]：分配本线程 TLS 块（用户堆一页，返回基址）
//! - [`base`]：读本线程 tp（= 本线程 TLS 块基址）
//!
//! 装配点（谁把 tp 指向块）：
//! - 主线程：`entry::_start` 经 [`bootstrap`] 装配（main 前 tp 生效）
//! - 子线程：`task::closure` 构造 [`TaskArg`] 携带块地址，`uktask_trampoline`
//!   出生时 `mv tp` 装配（每线程一帧，trap 保存/恢复自重）
//!
//! 与内核约定正交：内核态 tp = PerHart 指针（trap 入口重建），U 态 tp =
//! 本线程 TLS 块——`__utrap` 存 / `__restore` 恢复全程保留用户 tp，互不干扰。

use crate::env;
use ubi::UResult;

/// TLS 块大小：一页（用户堆按页对齐分配；块内容用户自定布局）。
pub const TLS_SIZE: usize = crate::PAGE_SIZE;

/// 读本线程 tp（= 本线程 TLS 块基址）。
///
/// 只在装配点之后有效（主线程 bootstrap 后 / 子线程 trampoline 装配后）。
#[inline]
pub fn base() -> usize {
    let tp: usize;
    // SAFETY: 纯读 tp，无副作用。
    unsafe {
        core::arch::asm!("mv {0}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
    }
    tp
}

/// 分配一块 TLS 块（用户堆一页）并返回基址；装配由调用方（bootstrap /
/// 子线程 trampoline）把该地址写入 tp。
pub fn alloc() -> UResult<usize> {
    env::allocate(TLS_SIZE)
}

/// 主线程 TLS 装配：分配 TLS 块并把 tp 指向它。`entry::_start` 于 `main` 前调用。
///
/// # Safety
/// 仅在主线程出生点（`_start` → `main` 之间）调用恰好一次；此后本线程
/// `tp` = 本线程 TLS 块基址。
#[unsafe(no_mangle)]
pub extern "C" fn bootstrap() {
    let addr = alloc().expect("tls bootstrap alloc failed");
    // SAFETY: 写 tp（用户态可自由写该寄存器；本线程刚出生，无旧值需保留）。
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) addr, options(nomem, nostack, preserves_flags));
    }
}
