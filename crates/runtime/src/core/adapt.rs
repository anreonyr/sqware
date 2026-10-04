//! adapt — **调用方口径 → 内核口径**的那点转换。
//!
//! 全 runtime 只剩这一处做这种转换，其余每一格都是 `crates/env` 生成的入口，直接叫它：
//! `usize` ↔ `VirtAddr`、`&[T]` ↔ `(ptr, len)`、`Duration` → 毫秒（**向上取整**）、
//! `Option<&str>` → `(ptr, len)`。
//!
//! 每一条转换都只有一个理由（写在该条的注里）：它是**全树只有一处**的口径，散进调用点
//! 就变成 n 份各自记得住记不住的约定。
//!
//! **没有 `mmap` / `unmap` / `mprotect` / `backtrace`**：那三格今天零调用者，删了就删了
//! ——要再加回来，写在用它的那一格旁边。

use core::time::Duration;

use env::{Reason, RoomCall, TaskId, TeamId, UnitResult, VirtAddr};
use env::{MemoryResult, PieToken};

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

/// 用户堆分配（按页取整、至少一页）。
///
/// # Errors
/// - `OoM`(-2)      段耗尽 / 物理帧耗尽
/// - `NoRegion`(-4) 本域没有 `user` 段（不变量破了）
pub fn allocate(size: usize) -> MemoryResult<usize> {
    env::memory::allocate(size).map(|va| va.get())
}

/// 用户堆释放（`(addr, size)` 必须精确匹配本段已分配的块）。
///
/// # Errors
/// - `Denied`(-1) 这一区间不在本任务那张簿记里
pub fn deallocate(addr: usize, size: usize) -> MemoryResult<()> {
    env::memory::deallocate(VirtAddr::new(addr), size)
}

/// 安装页映射（`at = 0` 窗口自选；`size`/`offset` 按页对齐；`flags` 是 R/W/X 位 1/2/3）。
///
/// # Errors
/// - `OoM`(-2)           窗口自选时段不足
/// - `NotAligned`(-3)    定点 `at` 未页对齐
/// - `AlreadyMapped`(-5) 定点 `at` 已被映射
pub fn map(
    team: TeamId,
    at: usize,
    size: usize,
    backing: PieToken,
    offset: usize,
    flags: u64,
) -> MemoryResult<usize> {
    env::memory::mmap(team, VirtAddr::new(at), size, backing, offset, flags).map(|va| va.get())
}

/// 释放 mmap / 声明区域（当前域：`TeamId(0)`）。
///
/// # Errors
/// - `Denied`(-1) 这一区间不是本段的已分配块
pub fn munmap(addr: usize, size: usize) -> MemoryResult<()> {
    env::memory::munmap(TeamId::new(0), VirtAddr::new(addr), size)
}
