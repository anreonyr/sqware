//! Room 域：`RoomCall::*` 转发（调度词族）。
//!
//! 每格一个**精确签名的入口**（`env::room::*`，由 `#[derive(Envcall)]` 生成）：本层只留
//! "调用方口径 → 内核口径"的那点转换（`Duration` → 毫秒、`Wait` 原样）。错类型是**这一域
//! 的词汇**（`RoomFail`：`Dead` / `OoM` / `Busy`）。
//!
//! [`exit`] 是**唯一手写**的一格（`RoomCall::Reap`）：它发散（`!`），且在栈上拼 `&str`
//! 那两个参数——形状与"每格一个入口"不同，故标了 `#[manual]`。

use core::time::Duration;

use env::{Reason, RoomCall, RoomResult, TaskId, VirtAddr, Wait};

/// 让出处理器。
///
/// **不返 `Result`**：这一手在**内核里没有失败分支**——`RoomCall::Starve` 那一格直接就是
/// "切走"（`envcall/mod.rs`：`return current().starve()`），没有 `ret_err` 那一支。从前它
/// 写着 `EnvResult<()>` 而函数体 `let _ = …call(); Ok(())`：**签名许诺了一个永不到来的
/// 失败**，读签名的人会去写 `?`，而那是死代码。
pub fn starve() {
    env::room::starve()
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
    // 这一格标了 `#[manual]`（发散），故它是全仓唯一还用 `call()` 的地方。
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

/// 睡一段（相对）：**至少这么久**，向上取整到毫秒。
///
/// # Errors
/// - `OoM`(-2) 等待位备料失败（**本任务没挂起**——见 `envcall/mod.rs` 的 `Park` 那一支）
pub fn sleep(d: Duration) -> RoomResult<()> {
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
    // **不吞错**：这一手**会失败**——内核那一格在"备料失败（内存耗尽）"时当场答 `OoM`，
    // 而**本任务没挂起**。吞掉它就是"答 `Ok` 而根本没睡"：调用方以为睡过了，实际是空转。
    // 要不要紧由**调用点**说（它们今天一律 `let _ =`）——那才是政策该在的地方。
    env::room::park(millis.min(usize::MAX as u128) as usize)
}

/// 睡到**绝对点**（`at` = `chrono::clock()` 的纳秒基准）：**不早于 `at`，且至多晚一拍**；
/// `at` 已过 ⇒ **当场返回**（不挂起、不报错）。
///
/// 周期任务用它才不会漂：`next += period; sleep_until(next)?;` —— 迟到不累积。
/// 与 [`sleep`] 的分工：那个是"至少睡这么久"（相对），这个是"到某个时刻再回来"（绝对）。
/// # Errors
/// - `OoM`(-2) 等待位备料失败（**本任务没挂起**）
pub fn sleep_until(at: u64) -> RoomResult<()> {
    env::room::park_until(at)
}

/// 他杀：把 `task` 送进既有的死亡路径——与 [`exit`] 成对（**自杀 ↔ 他杀**）。
///
/// **语义是域粒度**：`task` 只是"指认域"的手柄，它所属的域连同子树一起走（同域的
/// 线程一并，不会剩半个域）。判据只有**判活**——**没有血缘门**：收一个域是"命令"，
/// 不是"血缘特权"。曾经那道传递门（目标域沿 `sire` 链可达本域）已随 `Build` 的 S 态门
/// 同一次分家删掉，"该不该收"归 `protocol::system` 的编排者。
///
/// 失败只有 `Dead`（本域词汇里的那一枚）：目标从未入册 / 已回收 / **它那个域里已经没有
/// 还没收尾的线程**。
/// **不等它回收**——要等用 [`crate::env::unit::join`]，且注意 `Join` 只在"收尾"之前
/// 答得出（入土之后它问不出"没了"与"从来没有过"的区别）。
///
/// **调用者**（今天三处，都不是政策服务）：`protocol::system` 的收尾路径（`system::server`
/// 的 `doom`，由 `service::stop` 与"起失败"那一支调）——**编排域**
/// 点名收掉一个子域，而这一刀按域粒度走——与
/// `harness/src/group.rs` 的收场那一手（台子把没醒的等待者收掉，那是**台子自己的**客人，不是政策）。
///
/// **照实记（第四处已删）**：原先还有一手 `system::board::bridge::shut()`（编排域点名收掉
/// 同域那枚板线程）。它已删：按域粒度那一刀收的正是**编排域自己**，代价是最后那句判词
/// `system: done` 永远够不到（见 `system/main.rs` 收尾那一格的照实记）。板线程随"域亡＝成员
/// 清零"一起走，不需要点名。
pub fn doom(task: TaskId) -> RoomResult<()> {
    env::room::doom(task)
}

/// # Errors
/// - `OoM`(-2) 等待位备料失败（**本任务没挂起**）
pub fn wait(key: usize, millis: Wait) -> RoomResult<()> {
    env::room::wait(key, millis)
}

/// 唤醒 `key` 上的等待者：答**有没有人可唤醒**（`false` = 没有等待者，内核当场置
/// `pend` 给下一次等待）。
///
/// **不返 `Result`**：内核那一格恒写这一枚 bool，没有失败支（生成的那一格标了
/// `#[infallible]`）。
pub fn wake(key: usize) -> bool {
    env::room::wake(key)
}
