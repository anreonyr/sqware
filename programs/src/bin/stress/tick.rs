//! tick — 压测台**各程序共用**的**测时与空转**（`churn` / `rig` / `busy` / `hang` / `load`
//! 五者各自 `#[path]` 声明一次，与 `board.rs` 的作法同：这些程序互不依赖，共享的只有这一份
//! "怎么量时间"）。
//!
//! # 域能读到的钟只有一格
//!
//! `Chrono::Ticks` 数是**内核的定时器中断**（`timer::tick()`），不是时基——实测它的间隔就是
//! 调度量子 100 ms。故本模块的"刻度"= 那个计数，`ms_per_tick` 由 `sleep` 量出来，
//! `iters_per_tick` 由空转一段数出来（**等一次完整的刻度间隔**，不是半格）。
//!
//! 这些程序量的是同一把尺：受害者据此把"在台上"做成 1 ms，台主据此把"点名时刻"扫到微秒。

#![allow(dead_code)]

use core::time::Duration;

use runtime::env::chrono;
use runtime::env::room;

/// 一段"在台上"的目标时长（毫秒）：与 `churn` 的睡眠段一样长 ⇒ 一半在台上、一半离核。
pub const BURST_MS: usize = 1;

/// 量刻度换换算时，一次空转的块大小。
const CHUNK: usize = 4_096;

/// 量 `iters_per_tick` 时最多数多少块（刻度要是一直不动也不至于挂住）。
const MAX_CHUNKS: usize = 4_096;

/// 空转 `n` 轮。读一个从不改的量再 `black_box` 掉——不然整段会被优化没。
pub fn spin(n: usize) {
    static SEED: usize = 11;
    let mut acc = SEED;
    for _ in 0..n {
        acc = acc.wrapping_mul(3).wrapping_add(1);
        core::hint::spin_loop();
    }
    core::hint::black_box(acc);
}

/// 空转**约** `iters` 轮。
pub fn spin_iters(iters: usize) {
    spin(iters);
}

fn now() -> usize {
    chrono::ticks().unwrap_or(0)
}

/// 量本机两件事：`(每毫秒的空转轮数, 每刻度多少毫秒)`。
///
/// 刻度 = 定时器中断计数：先量它一格有多久（用 `sleep` 当基准，**不假设**它是 100 ms），
/// 再空转着数满**一整格**（等计数从 `a` 走到 `a+1`），于是 `iters_per_tick` 是真的整格。
pub fn calibrate() -> (usize, usize) {
    // 一格多少毫秒：睡 200 ms，看计数动了几格。
    let t0 = now();
    let _ = room::sleep(Duration::from_millis(200));
    let t1 = now();
    let spent = t1.saturating_sub(t0).max(1);
    let ms_per_tick = (200 / spent).max(1);

    // 一整格多少轮：先等到"刚翻过一格"的那一刻，再从那里数到下一格。
    let a = wait_tick(now());
    let b = wait_tick(a);
    let mut iters = 0usize;
    for _ in 0..MAX_CHUNKS {
        if now() > b {
            break;
        }
        spin(CHUNK);
        iters += CHUNK;
    }
    let iters_per_tick = iters.max(1);
    (iters_per_tick / ms_per_tick, ms_per_tick)
}

/// 等到刻度计数**大于** `from`，返那一刻的计数。
fn wait_tick(from: usize) -> usize {
    for _ in 0..MAX_CHUNKS {
        let t = now();
        if t > from {
            return t;
        }
        spin(CHUNK);
    }
    now()
}
