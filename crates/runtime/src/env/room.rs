//! Room 域：`RoomCall::*` 转发（调度词族）。

use core::time::Duration;

use env::{EnvResult, RoomCall, RoomCallRet, TaskId, VirtAddr};

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
/// 为什么原因码长在 `Reap` 上、而不是另立一个"panic 调用"：**"域不可续"
/// 是域的判断，内核只需要"这个任务不再续跑 + 为什么"**。另立入口等于把域的策略写进
/// ABI，并让"任务终止"这条不变量在 ABI 里有两个出口——本仓一度就是这样（
/// `ControlCall::Panic`），现已收回。
///
/// 约定：`0` = 自愿/正常；`1..` 留给域自己的诊断编号（各 bin 用 `1`、`2`… 标明
/// 死在启动握手的哪一步）；内核自己用高位段（见 `kernel/src/runtime/switcher/trap.rs`
/// 的 `EXIT_FAULT`）。
pub fn exit_with(reason: usize) -> ! {
    exit_with_note(reason, "")
}

/// 同 [`exit_with`]，再带**一句话**（`Reap { note }`）：**"哪里算不下去"只有域知道**。
///
/// 内核在入口当场把它拷进栈上的定长缓冲（至多 `env::NOTE_MAX`，超出截断）并**自己打印**
/// ——不依赖任何服务活着：一个正在退场的域不该先去求一条活路。
/// panic 现场那句话由 `programs/src/entry.rs` 的 panic handler 在栈上拼好
/// （`file:line` 是编译器塞进只读段的字面量，不需要符号表）。
pub fn exit_with_note(reason: usize, note: &str) -> ! {
    let _ = RoomCall::Reap {
        reason,
        note: VirtAddr::new(note.as_ptr() as usize),
        len: note.len().min(env::NOTE_MAX),
    }
    .call();
    unsafe { core::hint::unreachable_unchecked() }
}

pub fn sleep(d: Duration) -> EnvResult<()> {
    // **向上取整到毫秒**：`Park{millis}` 是**下限族**（"至少这么久"），而 `Park{0}` 的
    // 语义是**让出一拍**、不是"睡 0 毫秒"。
    //
    // 照实记（修掉的那个坑）：旧写法是 `d.as_millis() as usize`（**向下取整**）⇒
    // `sleep(500µs)` 静默变成 `Park{0}` = 让出一拍；`sleep(1.5ms)` 变成 `Park{1}`，
    // 连"至少 1.5 ms"这个下限都没守住。向上取整才与下限族口径一致：
    // `500µs → 1`、`1.5ms → 2`、`1ms → 1`、`0 → 0`（零时长仍是"让出一拍"）。
    let mut ms = d.as_millis();
    if d.subsec_nanos() % 1_000_000 != 0 {
        ms += 1;
    }
    let _ = RoomCall::Park {
        millis: ms.min(usize::MAX as u128) as usize,
    }
    .call();
    Ok(())
}

/// 他杀：把 `task` 送进既有的死亡路径——与 [`exit_with`] 成对（**自杀 ↔ 他杀**）。
///
/// **语义是域粒度**：`task` 只是"指认域"的手柄，它所属的域连同子树一起走（同域的
/// 线程一并，不会剩半个域）。判据只有**判活**——**没有血缘门**：收一个域是"命令"，
/// 不是"血缘特权"。曾经那道传递门（目标域沿 `sire` 链可达本域）已随 `Build` 的 S 态门
/// 同一次分家删掉，"该不该收"归 `protocol::system` 的编排者。
///
/// 失败只有 `Dead`(-2)：目标从未入册 / 已回收 / **它那个域里已经没有还没收尾的线程**。
/// **不等它回收**——要等用 [`crate::env::unit::join`]，且注意 `Join` 只在"收尾"之前
/// 答得出（入土之后它问不出"没了"与"从来没有过"的区别）。
///
/// **调用者**（两处，都不是政策服务）：`protocol::system` 的收尾路径（`call::ruin`，由
/// `service::stop` 与"起失败"那一支调）与 `board::shut()`——root 点名收掉同域的板线程，
/// 而这一刀按域粒度走，收的是 **root 自己那个域**。
pub fn doom(task: TaskId) -> EnvResult<()> {
    let _ = RoomCall::Doom { task }.call()?;
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
