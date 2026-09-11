//! Room 域：`RoomCall::*` 转发（调度词族）。

use core::time::Duration;

use env::{EnvResult, RoomCall, RoomCallRet};

pub fn starve() -> EnvResult<()> {
    let _ = RoomCall::Starve.call();
    Ok(())
}

/// 正常结束（原因码 0）。
pub fn exit() -> ! {
    exit_with(0)
}

/// 带**原因码**结束本任务：内核只把它记进 trace，不解释语义。
///
/// 为什么原因码长在 `Reap` 上、而不是另立一个"panic 调用"（§10.36）：**"域不可续"
/// 是域的判断，内核只需要"这个任务不再续跑 + 为什么"**。另立入口等于把域的策略写进
/// ABI，并让"任务终止"这条不变量在 ABI 里有两个出口——本仓一度就是这样（
/// `ControlCall::Panic`），现已收回。
///
/// 约定：`0` = 自愿/正常；`1..` 留给域自己的诊断编号（各 bin 用 `1`、`2`… 标明
/// 死在启动握手的哪一步）；内核自己用高位段（见 `kernel/src/runtime/switcher/trap.rs`
/// 的 `EXIT_FAULT`）。
pub fn exit_with(reason: usize) -> ! {
    let _ = RoomCall::Reap { reason }.call();
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
