// ── 适配层：task ──
//
// 任务生成入队（TaskBuilder::spawn 收尾）。

use alloc::sync::Arc;

use crate::machine;
use crate::runtime::diagnose::trace::{self, EventKind, SchedEvent};
use crate::work::room::tie;
use crate::work::unit::task::Task;

use super::core::conductors;

/// 新任务入队收尾：入簿（Team.tasks，3）+ 入本 hart starved（1）+ PUSHED 计数；
/// 锁外唤醒 WFI 休眠核。
///
/// 锁序：Team.tasks 与调度锁顺序获取、不嵌套（无 3 → 1 方向）。
pub(crate) fn push(task: Arc<Task>) {
    let me = machine::hart_id();
    let tid = task.id;
    task.team.push_task(&task);
    conductors()[me].push(task);
    tie::push();
    trace::note(EventKind::Sched(SchedEvent::Spawn { tid }));
    // 新任务出现：喊醒 WFI 休眠核（可 steal 取活）
    tie::yell();
}