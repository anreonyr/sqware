// ── 适配层：utask ──
//
// 用户任务面——envcall 服务接缝：把 U 态环境调用翻译为调度核心操作，返回下一帧 PA。
// park / wait / wake / reap 走 messenger（事件队列），starve / run 走 scheduler（per-hart 调度）。
// 命名见 `scheduler/mod.rs`。

use core::time::Duration;

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::gate::GateError;

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
    messenger::mark_reaped();
    // 必须在取活（可能触发 done→halt）**之前**清空 reaped 队列——否则最后退出
    // 的任务会带着它的栈/trap 帧及团队地址空间滞留到关机断言，被误报为帧泄漏。
    messenger::clear_loop();
    run()
}

/// 事件等待入口（envcall Wait 调用）：直通核心的 [`messenger::wait`]。
/// `key` 为已合成的唤醒源（envcall 边界负责并入空间身份）。
pub fn wait(key: WakeKey, dur: Duration) -> Handoff<()> {
    messenger::wait(key, dur)
}

/// 事件唤醒入口（envcall Wake 调用）：给 `key` 投递信号；返回是否唤醒到等待者。
pub fn wake(key: WakeKey) -> bool {
    messenger::wake(key)
}

/// 等目标回收入口（envcall Join 调用）：已回收 / 仍在 → 当场结论；否则挂起。
/// 授权在 envcall 边界做，本层只碰核心。
pub fn join(tid: usize, dur: Duration) -> Result<Handoff<bool>, GateError> {
    messenger::join(tid, dur)
}

/// ktask 事件等待入口（asm 包装）：永久等一个键（`Duration::MAX`，无超时），
/// 只能被 `wake(key)` 解锁。同 [`wait`] 但用于内核任务上下文。
#[allow(dead_code)] // 内核线程面：暂无树内使用者（目录已移出内核）
pub fn wait_forever(key: WakeKey) -> usize {
    match messenger::wait(key, Duration::MAX) {
        // 信标已至（永久等待被满足）：内核线程面没有调用方，直接取活。
        Handoff::Resume(()) => run(),
        Handoff::Switch(pa) => pa,
    }
}
