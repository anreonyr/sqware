// ── 适配层：task ──
//
// 放行入队（`Task::release` 收尾）。

use alloc::sync::Arc;

use crate::work::room::conductor;
use crate::work::unit::task::Task;

use super::core::current;

/// 放行后的任务入本核就绪队列（`Held → Starved` 之后由 `Task::release` 调用）：
/// 只做「入队 + 踢醒」。
///
/// 簿记（`Team.tasks`）、未放行容器（`Team.held`）、产生计数（PUSHED）与 trace
/// 都在 `TaskBuilder::hold` 完成——**计数挂在产生处**，Held 被父域 kill 时
/// REAPED/PUSHED 仍配平（否则 `done()` 恒假，系统永不停机）。
pub(crate) fn push(task: Arc<Task>) {
    current().push(task);
    // 新任务出现：单点踢醒 1 个 WFI 休眠核（可 steal 取活；多核广播会触发
    // 雷鸣群，多 hart 同时抢源 L1 → cache 行乒乓）。
    conductor::kick();
}
