//! Chrono 域：`ChronoCall::*` 转发。
//!
//! **两格都不返 `EnvResult`**：内核那两条路只写时钟读数，整条路径没有失败支
//! （判据与那批格子的清单见 `env::ecall::EnvResult` 的注）。

use env::{ChronoCall, ChronoCallRet};

pub fn ticks() -> usize {
    match ChronoCall::Ticks.call() {
        Ok(ChronoCallRet::Ticks(t)) => t,
        _ => unreachable!(),
    }
}

/// 自启动基准的**纳秒标量**（单调）——与 `room::sleep_until` 的 `at` 同基准同单位。
pub fn clock() -> u64 {
    match ChronoCall::Clock.call() {
        Ok(ChronoCallRet::Clock(ns)) => ns,
        _ => unreachable!(),
    }
}
