//! task — **任务本地原语**：[`join`]（域内并发与结果回收）· [`args`]（启动参数面）·
//! [`heap`]（用户堆后端）· [`lock`]（同域互斥）· [`tls`]（每线程 TLS 块）。

pub mod args;
pub mod heap;
pub mod join;
pub mod lock;
pub mod tls;

use core::time::Duration;
use env::{Reason, RoomCall, TaskId, TeamId, UnitResult, VirtAddr};

/// 睡一段（相对）：**至少这么久**，向上取整到毫秒。
///
/// **向上取整**：`Park{millis}` 是**下限族**（"至少这么久"），而 `Park{0}` 的语义是
/// **让出一拍**、不是"睡 0 毫秒"。向下取整会破坏这一口径（`sleep(500µs)` 静默变成
/// `Park{0}` = 让出一拍；`sleep(1.5ms)` 变成 `Park{1}`，连"至少 1.5 ms"这个下限都没守住）。
/// 向上取整才一致：`500µs → 1`、`1.5ms → 2`、`1ms → 1`、`0 → 0`。
///
/// **不吞错**：这一手**会失败**——内核那一格在"备料失败（内存耗尽）"时当场答 `OoM`，
/// 而**本任务没挂起**。吞掉它就是"答 `Ok` 而根本没睡"：调用方以为睡过了，实际是空转。
/// 要不要紧由**调用点**说（它们今天一律 `let _ =`）。
///
/// # Errors
/// - `OoM`(-2) 等待位备料失败（**本任务没挂起**）
pub fn sleep(d: Duration) -> env::RoomResult<()> {
    let mut millis = d.as_millis();
    if d.subsec_nanos() % 1_000_000 != 0 {
        millis += 1;
    }
    env::room::park(millis.min(usize::MAX as u128) as usize)
}

/// 结束本任务：**原因码 + 可选的一句话**——全仓**唯一**的出口原语。
///
/// 为什么原因码长在 `Reap` 上、而不是另立一个"panic 调用"：**"域不可续"是域的判断，
/// 内核只需要"这个任务不再续跑 + 为什么"**。另立入口等于把域的策略写进 ABI，并让
/// "任务终止"这条不变量在 ABI 里有两个出口。
///
/// `reason` 的约定：`0` = 自愿/正常；`1..` 留给域自己的诊断编号（各 bin 用 `1`、`2`…
/// 标明死在启动握手的哪一步）；高位段归 ABI（[`env::EXIT_PANIC`] / [`env::EXIT_FAULT`]）。
///
/// `note` 是**"哪里算不下去"那一句**（只有域知道），`None` = 无话。内核在入口当场把它
/// 拷进栈上的定长缓冲（至多 [`env::NOTE_MAX`]，超出截断）并**自己打印**——不依赖任何
/// 服务活着：一个正在退场的域不该先去求一条活路。
pub fn exit(reason: Reason, note: Option<&str>) -> ! {
    let note = note.unwrap_or("");
    // `Reap` 发散，故这里是全仓唯一还用 `call()` 的地方。
    let _ = RoomCall::Reap {
        reason,
        note: VirtAddr::new(note.as_ptr() as usize),
        len: note.len().min(env::NOTE_MAX),
    }
    .call();
    // 内核的 `Reap` 分支**永不返回帧**（`kernel/src/runtime/switcher/envcall/mod.rs` 的
    // `Reap` 分支返空指针，由退场窄尾的 `quit` 收）。真破了这条不变量就走域内 panic 通道
    // （`programs/src/entry.rs` 的 panic handler），不落 UB。
    unreachable!("Reap 返回了：reason={reason}")
}

/// 创建 task（**Held**，未放行）：`team`（`TeamId(0)` = 当前域）+ `entry`（0 = 域默认
/// 入口）+ 启动参数（写入新任务栈顶，`a0`/`a1` 取回）+ 栈（0 = 默认）。
///
/// 切片 → `(ptr, len)` 是这一格的唯一转换：内核只收一段裸地址，而调用方手上是一片切片。
///
/// 产出的 task 不会先于 `embark` 运行——父方可先 `Accord` 授权。
pub fn spawn(team: TeamId, entry: usize, args: &[usize], stack: usize) -> UnitResult<TaskId> {
    env::unit::spawn(
        team,
        entry,
        VirtAddr::new(args.as_ptr() as usize),
        args.len(),
        stack,
    )
}
