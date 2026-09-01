//! Room 域：`RoomCall::*` 转发（调度词族）。

use core::time::Duration;

use ubi::{RoomCall, UArgs, UResult, Ucall, UcallBuilder};

pub fn starve() -> UResult<()> {
    let _ = UcallBuilder::new(Ucall::Room(RoomCall::Starve)).call();
    Ok(())
}

pub fn exit() -> ! {
    let _ = UcallBuilder::new(Ucall::Room(RoomCall::Reap)).call();
    unsafe { core::hint::unreachable_unchecked() }
}

pub fn sleep(d: Duration) -> UResult<()> {
    let args = UArgs { a0: d.as_millis() as usize, ..UArgs::default() };
    let _ = UcallBuilder::new(Ucall::Room(RoomCall::Park)).args(args).call();
    Ok(())
}

pub fn wait(key: usize, ms: usize) -> UResult<()> {
    let args = UArgs { a0: key, a1: ms, ..UArgs::default() };
    let _ = UcallBuilder::new(Ucall::Room(RoomCall::Wait)).args(args).call();
    Ok(())
}

pub fn wake(key: usize) -> UResult<usize> {
    let args = UArgs { a0: key, ..UArgs::default() };
    let (v0, _) = UcallBuilder::new(Ucall::Room(RoomCall::Wake)).args(args).call()?;
    Ok(v0)
}