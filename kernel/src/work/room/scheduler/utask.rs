// ── 适配层：utask ──
//
// 用户任务面——envcall 服务接缝：把 U 态环境调用翻译为调度核心操作，返回下一帧 PA。
// park / wait / wake / reap 走 messenger（事件队列），starve / run 走 scheduler（per-hart 调度）。
// 命名见 `scheduler/mod.rs`。

use core::time::Duration;

use crate::work::room::messenger::{self, Handoff, WaitKey};

use super::core::current;
use super::trap::run;

/// 主动让出入口（envcall Starve 调用）：无视剩余预算立即轮转。
pub fn starve() -> usize {
    current().starve()
}

/// 当前线程睡眠入口（envcall Park 调用）：换算 deadline →
/// messenger::park；本核 starved 空 → run() 取活。
pub fn park(duration: Duration) -> usize {
    match messenger::park(duration) {
        Some(pa) => pa,
        None => run(),
    }
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

/// 事件等待入口（envcall Wait 调用）：pend 存在 → 消费即回；否则阻塞挂起。
///
/// 返回**是否切走**：`None` = 未离核（调用方续用当前帧）；`Some(pa)` = 切到该帧
/// （本核已装槽的下一位，或本核空时 `run()` 取来的）。`key` 为已合成的事件键
/// （envcall 边界负责并入空间身份）。
pub fn wait(key: WaitKey, dur: Duration) -> Option<usize> {
    match messenger::wait(key, dur) {
        Handoff::Resume => None,
        Handoff::Switch(pa) => Some(pa),
        Handoff::Idle => Some(run()),
    }
}

/// 事件唤醒入口（envcall Wake 调用）：给 `key` 投递信号；返回是否唤醒到等待者。
pub fn wake(key: WaitKey) -> bool {
    messenger::wake(key)
}
