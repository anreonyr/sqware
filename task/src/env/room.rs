//! Room 域：`RoomCall::*` 转发（调度词族）。

use core::time::Duration;

use env::{EnvResult, RoomCall, RoomCallRet};

pub fn starve() -> EnvResult<()> {
    let _ = RoomCall::Starve.call();
    Ok(())
}

pub fn exit() -> ! {
    let _ = RoomCall::Reap.call();
    unsafe { core::hint::unreachable_unchecked() }
}

pub fn sleep(d: Duration) -> EnvResult<()> {
    let _ = RoomCall::Park {
        millis: d.as_millis() as usize,
    }
    .call();
    Ok(())
}

pub fn wait(key: usize, ms: usize) -> EnvResult<()> {
    let _ = RoomCall::Wait { key, millis: ms }.call();
    Ok(())
}

pub fn wake(key: usize) -> EnvResult<usize> {
    let r = RoomCall::Wake { key }.call()?;
    match r {
        RoomCallRet::Wake(woke) => Ok(woke as usize),
        _ => unreachable!(),
    }
}
