//! Chrono 域：`ChronoCall::*` 转发。
//!
//! **两格都不失败**（生成的入口标了 `#[infallible]`）：内核那两条路只写时钟读数。

pub fn ticks() -> usize {
    env::chrono::ticks()
}

/// 自启动基准的**纳秒标量**（单调）——与 `room::sleep_until` 的 `at` 同基准同单位。
pub fn clock() -> u64 {
    env::chrono::clock()
}
