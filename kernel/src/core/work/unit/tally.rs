//! 在世任务的记账：起了多少活、收了多少活；“活都收完了”那一问从这里答。
//!
//! 起一笔在任务真正进世界的那一刻（`task::PreparedTask::publish`），收一笔在收
//! 完壳的时候（`reap::bury`）；停机判据 `done` 问的就是这两笔账对不对得上。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static PUSHED: AtomicUsize = AtomicUsize::new(0);
static REAPED: AtomicUsize = AtomicUsize::new(0);
static ROOTED: AtomicBool = AtomicBool::new(false);

/// 记一笔：一枚任务进了世界。
pub(crate) fn push() {
    PUSHED.fetch_add(1, Ordering::Relaxed);
}

/// 记一笔：一枚任务收完了。
pub(crate) fn exit() {
    REAPED.fetch_add(1, Ordering::Relaxed);
}

/// 世界的根已经立起来了（boot 起过入口任务）。
pub(crate) fn rooted() {
    ROOTED.store(true, Ordering::Release);
}

/// 活都收完了没有：根立起来了，且起的那些都收完了。
pub(crate) fn done() -> bool {
    if !ROOTED.load(Ordering::Acquire) {
        return false;
    }
    let pushed = PUSHED.load(Ordering::Relaxed);
    pushed == 0 || REAPED.load(Ordering::Relaxed) == pushed
}

/// 读数：起了多少 / 收了多少。
pub(crate) fn counts() -> (usize, usize) {
    (
        PUSHED.load(Ordering::Relaxed),
        REAPED.load(Ordering::Relaxed),
    )
}
