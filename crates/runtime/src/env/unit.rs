//! Unit 域（class 1）：`UnitCall::*` 转发（`Build` 装域 / `Spawn` 产线程 / `Hatch`
//! 放行 / `Join` 等结束 / `Oust` 放下 / `Fall` 等落表 + 血缘观察）。
//!
//! **启动参数（`save_args` / `args`）不在这里**：那是**任务本地状态**（`Spawn` 那两格的
//! 读侧），不是一次 envcall 转发——已随其余任务本地原语搬去 `crate::core::unit`
//! （那一处新开"启动参数面"一节）。本文件因此只剩"一次调用一个函数"。

use env::{EnvResult, ProgramKind, TaskId, TeamId, UnitCall, UnitCallRet, VirtAddr};

/// 装域：镜像字节 + 特权级 → 新域（Space + Team，**无线程**）。
///
/// # 门
///
/// **没有门**——能不能起由内核回答（答"能"），该不该起归 `protocol::system` 的编排者
/// （那一侧的动词叫 `Mint`）。早先两道门都删了："存在权"门收一枚 `Nole`（而 `UnsealNole`
/// 自铸无代价，与"是 S 态"等价，是门形的装饰）；S 态门本身也删了。放开**不构成提权**：
/// 特权级由内核打包表决定（调用方说不上话），镜像仍要调用方交字节。
///
/// **字节不被拷走**：内核按段现读 `elf` 那几页（从前是整份拷进内核暂存）——故调用期间
/// 这段区间必须一直映射着，且**读完之前不许 `munmap`**（本域另一枚线程并发放手 ⇒ 恰好
/// 读不到的那几页答 `-1`）。
///
/// 失败 `-6 BadImage`（镜像不可装载）/ `-1 Denied`（镜像区读不出来）/ `-4 OoM`（头窗口
/// 或装载帧备不下）。
pub fn build(elf: &[u8], kind: ProgramKind) -> EnvResult<TeamId> {
    let r = UnitCall::Build {
        elf: VirtAddr::new(elf.as_ptr() as usize),
        len: elf.len(),
        kind,
    }
    .call()?;
    match r {
        UnitCallRet::Build(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 产线程（**Held**，未放行）：`team`（`TeamId(0)` = 当前域）+ `entry`（0 = 域默认
/// 入口）+ 启动参数（写入新任务栈顶，`a0`/`a1` 取回）+ 栈（0 = 默认）。
///
/// 产出的线程不会先于 [`hatch`] 运行——父方可先 `Accord` 授权。
pub fn spawn(team: TeamId, entry: usize, args: &[usize], stack: usize) -> EnvResult<TaskId> {
    let r = UnitCall::Spawn {
        team,
        entry,
        args: VirtAddr::new(args.as_ptr() as usize),
        count: args.len(),
        stack,
    }
    .call()?;
    match r {
        UnitCallRet::Spawn(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 放行（`Held → Starved`）。放行只发生一次——重复调用 → `-1 Denied`。
pub fn hatch(task: TaskId) -> EnvResult<()> {
    let _ = UnitCall::Hatch { task }.call()?;
    Ok(())
}

/// **放下**一个子域：摘掉我自己 `heir` 表里那一格。
///
/// 前置：那域里没有还没收尾的线程（否则 `-3 Busy`）；它必须是我生的（否则 `-1 Denied`）。
/// 一次一格；重复调用答 `Denied`。要等它收干净：先 `Doom { task }`（`task` 只是指认域的
/// 手柄），再按 `join` 的两段式循环等。
pub fn oust(team: TeamId) -> EnvResult<()> {
    let _ = UnitCall::Oust { team }.call()?;
    Ok(())
}

/// 等目标回收：`millis`（0 = 只探测，`usize::MAX` = 永久）。
///
/// `true` = **调用开始时**目标已回收（未挂起）；`false` = 未回收（可能挂起过）。
/// 调用模式：`loop { if join(task, 0)? { break } join(task, usize::MAX)? }`。
pub fn join(task: TaskId, millis: usize) -> EnvResult<bool> {
    let r = UnitCall::Join { task, millis }.call()?;
    match r {
        UnitCallRet::Join(b) => Ok(b),
        _ => unreachable!(),
    }
}

/// 等"我自己这张权限表里落进一枚"（`millis` 三态同全树）。
pub fn fall(millis: usize) -> EnvResult<bool> {
    let r = UnitCall::Fall { millis }.call()?;
    match r {
        UnitCallRet::Fall(b) => Ok(b),
        _ => unreachable!(),
    }
}

/// 当前 task id（0 = 无上下文）。
pub fn self_id() -> EnvResult<TaskId> {
    let r = UnitCall::SelfId.call()?;
    match r {
        UnitCallRet::SelfId(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 溯源：**生我者**的 task id（0 = 顶级域 / 父已亡）。
///
/// **层**：这一格问的是**任务血缘**（谁生了我）——与 `AnyPie::sire`（这枚门闩从哪一枚派生）
/// 同字不同层。
pub fn sire() -> EnvResult<TaskId> {
    let r = UnitCall::Sire.call()?;
    match r {
        UnitCallRet::Sire(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 我生的子域数量（heir 枚举 first pass）。
pub fn heir_count() -> EnvResult<usize> {
    let r = UnitCall::HeirCount.call()?;
    match r {
        UnitCallRet::HeirCount(n) => Ok(n),
        _ => unreachable!(),
    }
}

/// 按索引取子域 TeamId（heir 枚举 second pass；越界 → 0）。
pub fn heir_at(index: usize) -> EnvResult<TeamId> {
    let r = UnitCall::Heir { index }.call()?;
    match r {
        UnitCallRet::Heir(id) => Ok(id),
        _ => unreachable!(),
    }
}
