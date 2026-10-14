//! 停机路上的信标：世界的根没了就当家要散了，长时间没进展就报一次谁还在。
//!
//! `fetch` 靠 `shutting_down` 挑短的睡；`idle` 是那颗“报一次”的探针。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use env::TaskId;

use crate::core::work::unit::tally;
use crate::core::work::unit::task::Task;
use crate::machine::clock;

const STALL_MS: u64 = 2_000;

static LAST_PROGRESS: AtomicU64 = AtomicU64::new(0);
static LAST_COUNTS: AtomicU64 = AtomicU64::new(0);
static BEACON_FIRED: AtomicBool = AtomicBool::new(false);

static ROOT_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn arm(root: &Arc<Task>) {
    ROOT_ID.store(root.ident.id.get(), Ordering::Relaxed);
}

pub(crate) fn shutting_down() -> bool {
    let id = ROOT_ID.load(Ordering::Relaxed);
    if id == 0 {
        return false;
    }
    match super::super::scheduler::roster::muster(TaskId::new(id)) {
        None => true,
        Some(w) => match w.upgrade() {
            None => true,
            Some(t) => t.dead(),
        },
    }
}

fn stalled(pushed: usize, reaped: usize) -> bool {
    let now = clock::now().as_ticks();
    let packed = ((pushed as u64) << 32) | (reaped as u64 & 0xffff_ffff);
    if LAST_COUNTS.swap(packed, Ordering::Relaxed) != packed
        || LAST_PROGRESS.load(Ordering::Relaxed) == 0
    {
        LAST_PROGRESS.store(now, Ordering::Relaxed);
        return false;
    }
    let t0 = LAST_PROGRESS.load(Ordering::Relaxed);
    let past = now.wrapping_sub(t0);
    past >= clock::duration_to_ticks(core::time::Duration::from_millis(STALL_MS))
}

pub(crate) fn idle(hart: crate::machine::hart::HartId) {
    if BEACON_FIRED.load(Ordering::Relaxed) {
        return;
    }
    let (pushed, reaped) = tally::counts();
    if !shutting_down() {
        return;
    }
    if reaped == 0 || reaped >= pushed {
        LAST_PROGRESS.store(0, Ordering::Relaxed);
        return;
    }
    if !stalled(pushed, reaped) {
        return;
    }
    BEACON_FIRED.store(true, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    {
        let (holders, husks) = crate::core::work::room::messenger::probe_bookkeeping();
        let (more, ids) = super::super::scheduler::roster::roster_live_ids();
        let live = {
            let mut n = 0usize;
            while n < ids.len() && ids[n] != 0 {
                n += 1;
            }
            n
        };
        crate::putln!(
            "[stop] hart {hart} 空闲等待：PUSHED={pushed} REAPED={reaped}（差 {}）husks={husks} \
             holders={holders} 在世任务 id={:?}{}",
            pushed.saturating_sub(reaped),
            &ids[..live],
            if more > 0 { " …" } else { "" }
        );
    }
    #[cfg(not(debug_assertions))]
    {
        crate::putln!(
            "[stop] hart {hart} 空闲等待：PUSHED={pushed} REAPED={reaped}（差 {}）",
            pushed.saturating_sub(reaped)
        );
    }
}
