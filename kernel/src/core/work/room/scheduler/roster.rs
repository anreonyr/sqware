//! 在世任务名册：按 id 找得到每一枚还活着的任务。
//!
//! `publish` 在任务真正进世界那一刻登记（先留位、后插入，插不下就退回去）；
//! `prune_dead` 在收尸时把死掉的弱引用扫掉。

use alloc::sync::Arc;
use alloc::vec::Vec;

use hashbrown::HashMap;

use env::TaskId;

use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::task::{Task, TaskWeak};
use crate::machine::sync::{Level, OnceLock, SpinLock};

pub(super) static ROSTER: OnceLock<SpinLock<HashMap<TaskId, TaskWeak>>> = OnceLock::new();

fn roster_table() -> &'static SpinLock<HashMap<TaskId, TaskWeak>> {
    ROSTER.get_or_init(|| SpinLock::new_level(Level::Roster, HashMap::new()))
}

#[cfg(debug_assertions)]
static FAIL_RESERVE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_next_reservation() {
    FAIL_RESERVE.store(true, core::sync::atomic::Ordering::Release);
}

/// 先给这一枚任务留个位子：留不下就别往下构造了。
pub(crate) fn reserve_publication() -> Result<(), ()> {
    let mut roster = roster_table().lock();
    #[cfg(debug_assertions)]
    if FAIL_RESERVE.swap(false, core::sync::atomic::Ordering::AcqRel) {
        return Err(());
    }
    roster.try_reserve(1).map_err(|_| ())
}

pub(crate) fn publish<T>(commit: impl FnOnce() -> (Arc<Task>, T)) -> Result<(Arc<Task>, T), ()> {
    let mut roster = roster_table().lock();
    if roster.len() == roster.capacity() {
        return Err(());
    }
    #[cfg(debug_assertions)]
    let _commit = crate::core::memory::allocator::NoAllocation::enter();
    let (task, value) = commit();
    let id = task.ident.id;
    let previous = roster.insert(id, TaskWeak::stored(Arc::downgrade(&task), Origin::Roster));
    assert!(previous.is_none(), "publish: duplicate task id");
    Ok((task, value))
}

/// 按 id 取一份任务的弱引用（找不到或已死掉之外的事都不管）。
pub(crate) fn muster(id: TaskId) -> Option<TaskWeak> {
    roster_table()
        .lock()
        .get(&id)
        .map(|w| w.copy_at(Origin::Muster))
}

pub(crate) fn prune_dead() -> usize {
    let mut g = roster_table().lock();
    let before = g.len();
    g.retain(|_, w| w.live());
    before - g.len()
}

#[cfg(debug_assertions)]
pub(crate) fn roster_live_ids() -> (usize, [usize; 8]) {
    let g = roster_table().lock();
    let mut out = [0usize; 8];
    let mut n = 0usize;
    let mut more = 0usize;
    for (id, w) in g.iter() {
        if !w.live() {
            continue;
        }
        if n < 8 {
            out[n] = id.get();
            n += 1;
        } else {
            more += 1;
        }
    }
    (more, out)
}

pub(crate) fn roster() -> Vec<TaskWeak> {
    let g = roster_table().lock();
    let mut out: Vec<TaskWeak> = Vec::new();
    if out.try_reserve(g.len()).is_err() {
        return Vec::new();
    }
    out.extend(g.values().map(|w| w.copy_at(Origin::Snapshot)));
    out
}
