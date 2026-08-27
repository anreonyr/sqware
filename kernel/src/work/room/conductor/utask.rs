// ── 适配层：utask ──
//
// 用户任务面——envcall 服务接缝：把 U 态环境调用（Yield/Sleep/Exit）翻译为
// 调度核心操作，返回下一帧 PA（切换由陷阱机械沿返回链完成）。内核任务面
// （ktask）内部复用同一接线。
//
// 命名：与核心/内核面同词（park/starve/reap），路径 + 签名区分——
//   `Conductor::park`(核心方法) / `ktask::park`(内核面，软陷阱) / 本模块(用户面)。

use core::time::Duration;

use crate::machine;

use super::core::{clear, conductors};
use super::trap::run;

/// 主动让出入口（envcall YIELD 调用）：无视剩余预算立即轮转。
pub fn starve() -> usize {
    let me = machine::hart_id();
    conductors()[me].starve()
}

/// 当前线程睡眠入口（envcall ENV_SLEEP/ENV_MSLEEP 分支调用）：换算 deadline →
/// 方法 park；本核 starved 空 → run() 取活。
pub fn park(duration: Duration) -> usize {
    match conductors()[machine::hart_id()].park(duration) {
        Some(pa) => pa,
        None => run(),
    }
}

/// 当前线程退出入口（envcall ENV_EXIT 分支调用）：标记 Reaped + 取下一任务
/// （run 的取活循环；拿不到就 WFI）；全部任务退出 → halt。
pub fn reap() -> usize {
    let me = machine::hart_id();
    conductors()[me].reap();
    // 回收 Reaped：此刻执行在 per-hart trap 栈上，不触碰任务内存；Reaped 任务
    // 不在任何核运行（running/starved 均无引用），任意核回收均安全。
    //
    // 必须在取活（可能触发 done→halt）**之前**清空 reaped 队列——否则最后退出
    // 的任务会带着它的栈/trap 帧及团队地址空间滞留到关机断言，被误报为帧泄漏
    // （reap 自身先入队，clear 后置时 run() 一旦走 done→halt 路径 clear 永不执行）。
    clear();
    // 取下一任务：此刻 running 已 take，本核空闲 → run（steal / WFI）
    run()
}