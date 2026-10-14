//! 本核的运行环：trap 返回之后，这一步决定接下来跑谁。

use crate::core::work::room::messenger;
use crate::machine::stop::hush;

use super::current;
use super::fetch::fetch;

/// trap 返回时的下一步：返回下一个任务的帧地址。
pub(crate) fn run() -> usize {
    hush();
    let commit = crate::core::work::commit();
    if current()
        .running_task()
        .is_some_and(|task| messenger::take_doomed(task.ident.id).is_some())
    {
        drop(commit);
        return messenger::quit();
    }
    let next = current().advance();
    drop(commit);
    match next {
        Some(pa) => pa,
        None => fetch(),
    }
}
