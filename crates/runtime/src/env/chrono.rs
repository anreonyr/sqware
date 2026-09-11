//! Chrono 域：`ChronoCall::*` 转发。

use env::{ChronoCall, ChronoCallRet, EnvResult};

pub fn ticks() -> EnvResult<usize> {
    let r = ChronoCall::Ticks.call()?;
    match r {
        ChronoCallRet::Ticks(t) => Ok(t),
        _ => unreachable!(),
    }
}

pub fn clock() -> EnvResult<(u64, u64)> {
    let r = ChronoCall::Clock.call()?;
    match r {
        ChronoCallRet::Clock(secs) => Ok(secs),
        _ => unreachable!(),
    }
}
