// ── 适配层：utask ──
//
// 用户任务面——envcall 服务接缝：把 U 态环境调用翻译为调度核心操作，返回下一帧 PA。
// 命名见 `conductor/mod.rs`。

use core::time::Duration;

use crate::machine;

use super::core::{clear, conductors, WaitKey};
use super::trap::run;

/// 主动让出入口（envcall Starve 调用）：无视剩余预算立即轮转。
pub fn starve() -> usize {
    let me = machine::hart_id();
    conductors()[me].starve()
}

/// 当前线程睡眠入口（envcall Park 调用）：换算 deadline →
/// 方法 park；本核 starved 空 → run() 取活。
pub fn park(duration: Duration) -> usize {
    match conductors()[machine::hart_id()].park(duration) {
        Some(pa) => pa,
        None => run(),
    }
}

/// 当前线程退出入口（envcall Reap 调用）：标记 Reaped + 取下一任务
/// （run 的取活循环；拿不到就 WFI）；全部任务退出 → halt。
pub fn reap() -> usize {
    let me = machine::hart_id();
    conductors()[me].reap();
    // 必须在取活（可能触发 done→halt）**之前**清空 reaped 队列——否则最后退出
    // 的任务会带着它的栈/trap 帧及团队地址空间滞留到关机断言，被误报为帧泄漏。
    clear();
    run()
}

/// 事件等待入口（envcall Wait 调用）：pend 存在 → 消费即回；否则阻塞挂起。
///
/// 返回下一帧 PA（与 park 同契约：阻塞即切走，唤醒后从调用点「第二次返回」）。
/// `key` 为已合成的事件键（envcall 边界负责并入空间身份）。
pub fn wait(key: WaitKey, dur: Duration) -> usize {
    match conductors()[machine::hart_id()].wait(key, dur) {
        Some(pa) => pa,
        None => run(),
    }
}

/// 事件唤醒入口（envcall Wake 调用）：给 `key` 投递信号；返回是否唤醒到等待者。
pub fn wake(key: WaitKey) -> bool {
    super::core::wake(key)
}