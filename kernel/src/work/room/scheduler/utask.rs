// ── 适配层：utask ──
//
// 用户任务面——envcall 服务接缝：把 U 态环境调用翻译为调度核心操作，返回下一帧 PA。
// park / wait / wake / reap 走 messenger（事件队列），starve / run 走 scheduler（per-hart 调度）。
// 命名见 `scheduler/mod.rs`。

use alloc::sync::Weak;
use core::time::Duration;

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::gate::GateError;
use crate::work::unit::life::{Life, TaskLife};

use super::core::current;
use super::trap::run;

/// 主动让出入口（envcall Starve 调用）：无视剩余预算立即轮转。
pub fn starve() -> usize {
    current().starve()
}

/// 当前线程睡眠入口（envcall Park 调用）：纯睡——键是 `Alarm{我}`，必挂起，
/// 返回值就是下一帧（本核无后继时由核心内部取活）。
pub fn park(duration: Duration) -> usize {
    messenger::park(duration)
}

/// 当前线程退出入口（envcall Reap 调用）：标记 Reaped + 取下一任务
/// （run 的取活循环；拿不到就 WFI）；全部任务退出 → halt。
pub fn reap() -> usize {
    messenger::quit();
    // 必须在取活（可能触发 done→halt）**之前**清空躯壳队列——否则最后退出的
    // 任务会带着它的栈/trap 帧及团队地址空间滞留到关机断言，被误报为帧泄漏。
    messenger::bury();
    run()
}

/// 事件等待入口（envcall Wait 调用）：直通核心的 [`messenger::wait`]。
/// `key` 为已合成的唤醒源（envcall 边界负责并入空间身份）；`life` 为**该键的存活
/// 单元**（同样由调用方解析——room 不查任何注册表）。
pub fn wait(key: WakeKey, life: &Weak<Life>, dur: Duration) -> Handoff<()> {
    messenger::wait(key, life, dur)
}

/// 事件唤醒入口（envcall Wake 调用）：给 `key` 投递信号；返回是否唤醒到等待者。
/// 键已死 ⇒ `false`（资源没了，这个键不再有等待者）。
pub fn wake(key: WakeKey, life: &Weak<Life>) -> bool {
    messenger::wake(key, life)
}

/// 等目标回收入口（envcall Join 调用）：已回收 / 仍在 → 当场结论；否则挂起。
/// 授权与**键→存活单元的解析**都在 envcall 边界做（那里本来就握着目标的
/// `Arc<Task>`），本层只碰核心。
pub fn join(task: TaskLife, dur: Duration) -> Result<Handoff<bool>, GateError> {
    messenger::join(task, dur)
}

/// ktask 事件等待入口（asm 包装）：永久等一个键（`Duration::MAX`，无超时），
/// 只能被 `wake(key)` 解锁。同 [`wait`] 但用于内核任务上下文。
///
/// 注：包它的 asm（`ktask::wait_forever`）当前与本签名不符（只递 a0，且来源是裸
/// `usize` 而非 `WakeKey`），该路径树内零调用者——失效说明与处置见
/// `docs/audit-flying-wires.md` §D3。
#[allow(dead_code)] // 内核线程面：暂无树内使用者（目录已移出内核）
pub fn wait_forever(key: WakeKey, life: &Weak<Life>) -> usize {
    match messenger::wait(key, life, Duration::MAX) {
        // 信标已至（永久等待被满足）：内核线程面没有调用方，直接取活。
        Handoff::Resume(()) => run(),
        Handoff::Switch(pa) => pa,
    }
}
