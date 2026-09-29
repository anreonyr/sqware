use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use env::TaskId;

use crate::work::unit::task::Task;

const STALL_MS: u64 = 2_000;

static LAST_PROGRESS: AtomicU64 = AtomicU64::new(0);
static LAST_COUNTS: AtomicU64 = AtomicU64::new(0);
static BEACON_FIRED: AtomicBool = AtomicBool::new(false);

static ROOT_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn arm(root: &Arc<Task>) {
    ROOT_ID.store(root.ident.id.get(), Ordering::Relaxed);
}

pub(super) fn shutting_down() -> bool {
    let id = ROOT_ID.load(Ordering::Relaxed);
    if id == 0 {
        return false;
    }
    match super::table::muster(TaskId::new(id)) {
        None => true,
        Some(w) => match w.upgrade() {
            None => true,
            Some(t) => matches!(
                t.tag(),
                crate::work::unit::task::TaskTag::Reaped | crate::work::unit::task::TaskTag::Doomed
            ),
        },
    }
}

fn stalled(pushed: usize, reaped: usize) -> bool {
    let now = crate::runtime::chrono::clock::now().as_ticks();
    let packed = ((pushed as u64) << 32) | (reaped as u64 & 0xffff_ffff);
    if LAST_COUNTS.swap(packed, Ordering::Relaxed) != packed
        || LAST_PROGRESS.load(Ordering::Relaxed) == 0
    {
        LAST_PROGRESS.store(now, Ordering::Relaxed);
        return false;
    }
    let t0 = LAST_PROGRESS.load(Ordering::Relaxed);
    let past = now.wrapping_sub(t0);
    past >= crate::runtime::chrono::clock::duration_to_ticks(core::time::Duration::from_millis(
        STALL_MS,
    ))
}

pub(super) fn idle(hart: crate::hart::HartId) {
    if BEACON_FIRED.load(Ordering::Relaxed) {
        return;
    }
    let (pushed, reaped) = crate::work::room::conductor::counts();
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
        let (holders, husks) = crate::work::room::messenger::probe_bookkeeping();
        let (more, ids) = super::table::roster_live_ids();
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
