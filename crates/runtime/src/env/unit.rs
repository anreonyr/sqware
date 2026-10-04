//! Unit 域（class 1）：`UnitCall::*` 转发（`Build` 装域 / `Spawn` 创建 task / `Embark`
//! 放行 / `Join` 等结束 / `Oust` 放下 / `Fall` 等落表 + 血缘观察）。
//!
//! **启动参数（`save_args` / `args`）不在这里**：那是**任务本地状态**（`Spawn` 那两格的
//! 读侧），不是一次 envcall 转发——已随其余任务本地原语搬去 `crate::core::task`
//! （那一处新开"启动参数面"一节）。本文件因此只剩"一次调用一个函数"。

use env::{Wait, ProgramKind, TaskId, TeamId, UnitResult, VirtAddr};

/// 创建空的 Constructing 域。
pub fn build(kind: ProgramKind) -> UnitResult<TeamId> {
    env::unit::build(kind)
}

/// 创建 task（**Held**，未放行）：`team`（`TeamId(0)` = 当前域）+ `entry`（0 = 域默认
/// 入口）+ 启动参数（写入新任务栈顶，`a0`/`a1` 取回）+ 栈（0 = 默认）。
///
/// 产出的 task不会先于 [`embark`] 运行——父方可先 `Accord` 授权。
pub fn spawn(team: TeamId, entry: usize, args: &[usize], stack: usize) -> UnitResult<TaskId> {
    env::unit::spawn(
        team,
        entry,
        VirtAddr::new(args.as_ptr() as usize),
        args.len(),
        stack,
    )
}

/// 首次放行或恢复指定 task。
pub fn embark(task: TaskId) -> UnitResult<()> {
    env::unit::embark(task)
}

/// **放下**一个子域：摘掉我自己 `heir` 表里那一格。
///
/// 前置：那域里没有还没收尾的 task（否则 `-2 Busy`）；它必须是我生的（否则 `-1 Denied`）。
/// 一次一格；重复调用答 `Denied`。要等它收干净：先 `Doom { task }`（`task` 只是指认域的
/// 手柄），再按 `join` 的两段式循环等。
pub fn oust(team: TeamId) -> UnitResult<()> {
    env::unit::oust(team)
}

/// 等目标回收：`millis`（上限族，`Wait`）。
///
/// `true` = **调用开始时**目标已回收（未挂起）；`false` = 未回收（可能挂起过）。
/// 调用模式：`loop { if join(task, Wait::POLL)? { break } join(task, Wait::Forever)? }`。
pub fn join(task: TaskId, millis: Wait) -> UnitResult<bool> {
    env::unit::join(task, millis)
}

/// 等"我自己这张权限表里落进一枚"（`millis` 上限族，同全树）。
pub fn fall(millis: Wait) -> UnitResult<bool> {
    env::unit::fall(millis)
}

/// 当前 task id（0 = 无上下文）。
///
/// **不返 `Result`**：内核那一格恒写 id（无上下文也是 0），没有失败支（生成的那一格标了
/// `#[infallible]`）。
pub fn self_id() -> TaskId {
    env::unit::self_id()
}

/// 溯源：**生我者**的 task id（0 = 顶级域 / 父已亡）。
///
/// **层**：这一格问的是**任务血缘**（谁生了我）——与 `AnyPie::sire`（这枚门闩从哪一枚派生）
/// 同字不同层。
///
/// **不返 `Result`**：同 [`self_id`]——内核恒写 id（顶级域 / 父已亡也是 0）。
pub fn sire() -> TaskId {
    env::unit::sire()
}

/// 我生的子域数量（heir 枚举 first pass）。
///
/// **不返 `Result`**：无上下文那一路也答 0（见 [`self_id`]）。
pub fn heir_count() -> usize {
    env::unit::heir_count()
}

/// 按索引取子域 TeamId（heir 枚举 second pass；越界 → 0）。
///
/// **不返 `Result`**：越界也是 0（见 [`self_id`]）——"没有这一格"与"不在上下文里"
/// 用同一条哨兵，故没有失败域。
///
/// **（今天没有调用者）**：本手与 `UnitCall::Heir` 这一格今天全仓无人用
/// （`protocol/src/system/mod.rs` 记着"枚举出来的域在三条动作面上仍是死端"）。
/// 留着是因为它是那套枚举的第二趟，不是"备复用"。
pub fn heir_at(index: usize) -> TeamId {
    env::unit::heir(index)
}

/// 请求挂起 task；Busy 表示目标正在离开处理器，调用者需重试确认。
pub fn debark(task: TaskId) -> UnitResult<()> {
    env::unit::debark(task)
}

/// 销毁指定 task 及其拥有的子 team。
pub fn slay(task: TaskId) -> UnitResult<()> {
    env::unit::slay(task)
}
