//! Room 域：`RoomCall::*` 转发（调度词族）。

use env::Wait;
use core::time::Duration;

use env::{EnvResult, Reason, RoomCall, RoomCallRet, TaskId, VirtAddr};

/// 让出处理器。
///
/// **返 `()`，不返 `EnvResult`**：这一手在**内核里没有失败分支**——`RoomCall::Starve` 那一格
/// 直接就是"切走"（`envcall/mod.rs`：`return current().starve()`），没有 `ret_err` 那一支。
/// 从前它写着 `EnvResult<()>` 而函数体 `let _ = …call(); Ok(())`：**签名许诺了一个永不到来的
/// 失败**，读签名的人会去写 `?`，而那是死代码。
pub fn starve() {
    // 唯一能答 `Err` 的是"这一问读不懂"（调用号不对）——那一格由**生成的**调用点排掉，
    // 不是这一手的失败域（同 `Reap` 之后那句 `unreachable!` 的口径：破的是不变量，不是失败）。
    let _ = RoomCall::Starve.call();
}

/// 结束本任务：**原因码 + 可选的一句话**——全仓**唯一**的出口原语。
///
/// 为什么原因码长在 `Reap` 上、而不是另立一个"panic 调用"：**"域不可续"
/// 是域的判断，内核只需要"这个任务不再续跑 + 为什么"**。另立入口等于把域的策略写进
/// ABI，并让"任务终止"这条不变量在 ABI 里有两个出口——本仓一度就是这样（
/// `ControlCall::Panic`），现已收回。
///
/// `reason` 的约定：`0` = 自愿/正常；`1..` 留给域自己的诊断编号（各 bin 用 `1`、`2`…
/// 标明死在启动握手的哪一步）；高位段归 ABI（[`env::EXIT_PANIC`] / [`env::EXIT_FAULT`]）。
///
/// `note` 是**"哪里算不下去"那一句**（只有域知道），`None` = 无话。内核在入口当场把它
/// 拷进栈上的定长缓冲（至多 [`env::NOTE_MAX`]，超出截断）并**自己打印**——不依赖任何
/// 服务活着：一个正在退场的域不该先去求一条活路。panic 现场那句话由
/// `programs/src/entry.rs` 的 panic handler 在栈上拼好（`file:line` 是编译器塞进只读段
/// 的字面量，不需要符号表）。
///
/// 名与形状照 `std::process::exit(code)`，第二个参数就是本仓加的那句话。
///
/// **照实记（合并前）**：这里是三个函数（`exit` / `exit_with` / `exit_with_note`），
/// note 那个是前两者的下半。
pub fn exit(reason: Reason, note: Option<&str>) -> ! {
    let note = note.unwrap_or("");
    let _ = RoomCall::Reap {
        reason,
        note: VirtAddr::new(note.as_ptr() as usize),
        len: note.len().min(env::NOTE_MAX),
    }
    .call();
    // 内核的 `Reap` 分支**永不返回帧**（`kernel/src/runtime/switcher/envcall/mod.rs`
    // 的 `Reap` 分支返空指针，由退场窄尾的 `quit` 收）。真破了这条不变量就走域内
    // panic 通道（`programs/src/entry.rs` 的 panic handler），不落 UB。
    unreachable!("Reap 返回了：reason={reason}")
}

pub fn sleep(d: Duration) -> EnvResult<()> {
    // **向上取整到毫秒**：`Park{millis}` 是**下限族**（"至少这么久"），而 `Park{0}` 的
    // 语义是**让出一拍**、不是"睡 0 毫秒"。
    //
    // 照实记（修掉的那个坑）：旧写法是 `d.as_millis() as usize`（**向下取整**）⇒
    // `sleep(500µs)` 静默变成 `Park{0}` = 让出一拍；`sleep(1.5ms)` 变成 `Park{1}`，
    // 连"至少 1.5 ms"这个下限都没守住。向上取整才与下限族口径一致：
    // `500µs → 1`、`1.5ms → 2`、`1ms → 1`、`0 → 0`（零时长仍是"让出一拍"）。
    let mut millis = d.as_millis();
    if d.subsec_nanos() % 1_000_000 != 0 {
        millis += 1;
    }
    // **`?`，不是 `let _ =`**：这一手**会失败**——内核那一格在"备料失败（内存耗尽）"时
    // 当场答 `OoM`，而**本任务没挂起**（见 `envcall/mod.rs` 的 `Park` 那一支）。吞掉它
    // 就是"答 `Ok` 而根本没睡"：调用方以为睡过了，实际是空转。要不要紧由**调用点**说
    // （它们今天一律 `let _ =`）——那才是政策该在的地方。
    RoomCall::Park {
        millis: millis.min(usize::MAX as u128) as usize,
    }
    .call()?;
    Ok(())
}

/// 睡到**绝对点**（`at` = `chrono::clock()` 的纳秒基准）：**不早于 `at`，且至多晚一拍**；
/// `at` 已过 ⇒ **当场返回**（不挂起、不报错）。
///
/// 周期任务用它才不会漂：`next += period; sleep_until(next)?;` —— 迟到不累积。
/// 与 [`sleep`] 的分工：那个是"至少睡这么久"（相对），这个是"到某个时刻再回来"（绝对）。
pub fn sleep_until(at: u64) -> EnvResult<()> {
    // 同 [`sleep`]：**会失败**（`ParkUntil` 那一支同样会因备料失败答 `OoM`，而本任务没挂起）
    // ⇒ 不吞。
    RoomCall::ParkUntil { at }.call()?;
    Ok(())
}

/// 他杀：把 `task` 送进既有的死亡路径——与 [`exit`] 成对（**自杀 ↔ 他杀**）。
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
/// **调用者**（今天三处，都不是政策服务）：`protocol::system` 的收尾路径（`call::doom`，
/// 由 `service::stop` 与"起失败"那一支调）、`system/board` 那一台的 `shut()`——**编排域**
/// 点名收掉同域的板线程，而这一刀按域粒度走，收的是**编排域自己那个域**——与
/// `harness/src/group.rs` 的收场那一手（台子把没醒的等待者收掉，那是**台子自己的**客人，不是政策）。
pub fn doom(task: TaskId) -> EnvResult<()> {
    let _ = RoomCall::Doom { task }.call()?;
    Ok(())
}

pub fn wait(key: usize, millis: Wait) -> EnvResult<()> {
    // 同 [`sleep`]：**会失败**（`Wait` 那一支的备料同样会答 `OoM`，而本任务没挂起）⇒ 不吞。
    RoomCall::Wait { key, millis }.call()?;
    Ok(())
}

pub fn wake(key: usize) -> EnvResult<usize> {
    let r = RoomCall::Wake { key }.call()?;
    match r {
        RoomCallRet::Wake(woke) => Ok(woke as usize),
        _ => unreachable!(),
    }
}
