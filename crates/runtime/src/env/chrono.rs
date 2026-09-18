//! Chrono 域：`ChronoCall::*` 转发。

use env::{ChronoCall, ChronoCallRet, EnvResult};

pub fn ticks() -> EnvResult<usize> {
    let r = ChronoCall::Ticks.call()?;
    match r {
        ChronoCallRet::Ticks(t) => Ok(t),
        _ => unreachable!(),
    }
}

/// 自启动基准的**纳秒标量**（单调）——与 `room::sleep_until` 的 `at` 同基准同单位。
pub fn clock() -> EnvResult<u64> {
    let r = ChronoCall::Clock.call()?;
    match r {
        ChronoCallRet::Clock(ns) => Ok(ns),
        _ => unreachable!(),
    }
}
