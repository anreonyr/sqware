// ── 适配层：trap ──
//
// 陷阱路径入口：run——有 running 走本核时间片推进（`advance`：续跑 / 轮转），
// 空槽走取活（`fetch`）。两个都是核心的原语，本面只剩「多核 panic 就地卧倒」+
// 「二选一转发」，不含任何调度策略。

use crate::runtime::diagnose::halt::hush;
use crate::work::room::scheduler::core::{current, fetch};

/// 统一入口：返回下一帧 PA（`restore` 的落点）。
pub fn run() -> usize {
    // 多核 panic：警报已拉响且本 hart 非报警源 → 就地卧倒（不返回）。
    // 覆盖空闲/WFI 核经 fetch 在**内核态**处理 IPI 唤醒的路径；常运行时恒 no-op。
    hush();
    match current().advance() {
        Some(pa) => pa,
        None => fetch(),
    }
}
