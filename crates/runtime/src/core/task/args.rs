//! task::args — **启动参数面**：`save_args` / `args`（`Spawn` 那两格的读侧，任务本地状态）。
//! `save_args` 只有汇编一个调用者：`programs/src/entry.rs` 的 `_start` 用 `call save_args`
//! 直接叫它（`#[unsafe(no_mangle)]` 就是为这一手——路径搬到哪儿，符号都不动）。

use core::sync::atomic::{AtomicUsize, Ordering};

// ── 启动参数面 ────────────────────────────────────────────────────────────
//
// **`save_args` 只有汇编一个调用者**：`programs/src/entry.rs` 的 `_start` 用
// `call save_args` 直接叫它（`#[unsafe(no_mangle)]` 就是为这一手——路径搬到哪儿，
// 符号都不动）。故它必须**在任何 Rust 调用之前**就能跑：这里只写两个静态，不碰 TLS。

/// 启动参数区 VA / 字数（`_start` 保存，见 [`args`]）。
static ARGS: AtomicUsize = AtomicUsize::new(0);
static ARG_COUNT: AtomicUsize = AtomicUsize::new(0);

/// `_start` 保存启动参数（a0 = args VA、a1 = count）——必须在任何调用之前。
#[unsafe(no_mangle)]
pub extern "C" fn save_args(args: usize, count: usize) {
    ARGS.store(args, Ordering::Relaxed);
    ARG_COUNT.store(count, Ordering::Relaxed);
}

/// 启动参数（`Spawn` 写入新任务栈顶的标量数组；空 = 无参数）。
pub fn args() -> &'static [usize] {
    let n = ARG_COUNT.load(Ordering::Relaxed);
    if n == 0 {
        return &[];
    }
    // SAFETY: 内核在 spawn 时把 n 个字写在本任务栈顶；本任务存活期间该区间有效。
    unsafe { core::slice::from_raw_parts(ARGS.load(Ordering::Relaxed) as *const usize, n) }
}
