//! Unit 域：`UnitCall::*` 转发（`Build` 装域 / `Spawn` 产线程 / `Hatch` 放行 /
//! `Join` 等结束 + 血缘观察）。

use env::{EnvResult, ProgramKind, TaskId, TeamId, UnitCall, UnitCallRet, VirtAddr};

use core::sync::atomic::{AtomicUsize, Ordering};

/// 启动参数区 VA / 字数（`_start` 保存，见 [`args`]）。
static ARGS: AtomicUsize = AtomicUsize::new(0);
static ARG_COUNT: AtomicUsize = AtomicUsize::new(0);

/// `_start` 保存启动参数（a0 = args VA、a1 = count）——必须在任何调用之前。
#[unsafe(no_mangle)]
pub extern "C" fn save_args(args: usize, count: usize) {
    ARGS.store(args, Ordering::Relaxed);
    ARG_COUNT.store(count, Ordering::Relaxed);
}

/// 启动参数（`Spawn` 写入新任务栈顶的标量数组；空 = 无参数）。
pub fn args() -> &'static [usize] {
    let n = ARG_COUNT.load(Ordering::Relaxed);
    if n == 0 {
        return &[];
    }
    // SAFETY: 内核在 spawn 时把 n 个字写在本任务栈顶；本任务存活期间该区间有效。
    unsafe { core::slice::from_raw_parts(ARGS.load(Ordering::Relaxed) as *const usize, n) }
}

/// 装域：镜像字节 + 特权级 + 名字 → 新域（Space + Team，**无线程**）。
///
/// # 两道门（都要过）
///
/// 1. `build` = **建域权**：调用方自己表里一枚活着的 `Nole`（`NolePie`，见
///    [`crate::env::mail::NolePie`]）。典型形态是启动时解封一枚、此后一直用；
///    它也可经 `Accord` 转授给别人——权威因此可审计、可撤销。
/// 2. 调用方仍须是 S 态（血缘树不进沙箱外，理由见 `env::fid` 该 variant）。
///
/// 名字 ≤ 31 字节。失败 `-6 BadImage`（镜像不可装载）/ `-1 Denied`（门没过）。
pub fn build(elf: &[u8], kind: ProgramKind, name: &str, build: &crate::env::mail::NolePie) -> EnvResult<TeamId> {
    let r = UnitCall::Build {
        elf: VirtAddr::new(elf.as_ptr() as usize),
        len: elf.len(),
        kind,
        name: VirtAddr::new(name.as_ptr() as usize),
        name_len: name.len(),
        build: env::PieToken::new(build.token()),
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

/// 当前 task id（0 = 无上下文）。
pub fn self_id() -> EnvResult<TaskId> {
    let r = UnitCall::SelfId.call()?;
    match r {
        UnitCallRet::SelfId(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 溯源：生我者的 task id（0 = 顶级域 / 父已亡）。
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
