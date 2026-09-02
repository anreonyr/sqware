//! Chrono 域：`ChronoCall::*` 转发。

use ubi::{ChronoCall, UResult, Ucall, UcallBuilder};

pub fn ticks() -> UResult<usize> {
    let (v0, _) = UcallBuilder::new(Ucall::Chrono(ChronoCall::Ticks)).call()?;
    Ok(v0)
}

pub fn clock() -> UResult<(u64, u64)> {
    let (secs, nanos) = UcallBuilder::new(Ucall::Chrono(ChronoCall::Clock)).call()?;
    Ok((secs as u64, nanos as u64))
}
