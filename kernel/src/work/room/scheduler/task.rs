// ── 适配层：task ──
//
// 任务生成入队（TaskBuilder::spawn 收尾）。

use alloc::sync::Arc;

use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::unit::task::Task;

use super::core::current;

/// 新任务入队收尾：入簿（Team.tasks，3）+ 入本 hart starved（1）+ PUSHED 计数；
/// 锁外唤醒 WFI 休眠核。
///
/// 锁序：Team.tasks 与调度锁顺序获取、不嵌套（无 3 → 1 方向）。
pub(crate) fn push(task: Arc<Task>) {
    let tid = task.ident.id;
    task.ident.team.push_task(&task);
    current().push(task);
    conductor::push();
    trace::note(EventKind::Room(RoomEvent::Spawn { tid }));
    // 新任务出现：喊醒 WFI 休眠核（可 steal 取活）
    conductor::yell();
}
