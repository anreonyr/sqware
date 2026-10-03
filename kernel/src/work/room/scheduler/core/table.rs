use alloc::sync::Arc;
use alloc::vec::Vec;

use hashbrown::HashMap;
use sbi::ecall::SArgs;
use sbi::{self, fid};

use env::TaskId;

use crate::hart::HartId;
use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::room::conductor;
use crate::work::room::messenger;
use crate::work::unit::task::Task;
use crate::work::unit::weak::{Site, TaskWeak};

use super::hart::Scheduler;

pub(in super::super) static SCHEDULERS: OnceLock<&'static [Scheduler]> = OnceLock::new();

pub(crate) fn rip() {
    let Some(cs) = SCHEDULERS.get() else { return };
    for c in cs.iter() {
        let mut i = c.inner.lock();
        c.starved_clear(&mut i);
    }
    for task in roster().into_iter().filter_map(|w| w.upgrade()) {
        task.boarding.lock().parked.take();
    }
    messenger::rip();
    if let Some(r) = ROSTER.get() {
        r.lock().clear();
    }
}

pub(super) fn schedulers() -> &'static [Scheduler] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

#[cfg(debug_assertions)]
pub(crate) fn scheduler_addr(i: HartId) -> usize {
    core::ptr::addr_of!(schedulers()[i.get()]) as usize
}

pub(crate) fn launch(task: Arc<Task>) {
    kick(conductor::pick(), task);
}

pub(crate) fn kick(hart: HartId, task: Arc<Task>) {
    schedulers()[hart.get()].push(task);
    if conductor::waiting(hart) {
        conductor::note_kick_ipi();
        let (word, bit) = hart.bit();
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: bit,
                a1: word * (usize::BITS as usize),
                ..Default::default()
            })
            .call();
    } else {
        conductor::note_fallback();
    }
}

static ROSTER: OnceLock<SpinLock<HashMap<TaskId, TaskWeak>>> = OnceLock::new();

fn roster_table() -> &'static SpinLock<HashMap<TaskId, TaskWeak>> {
    ROSTER.get_or_init(|| SpinLock::new_level(Level::Roster, HashMap::new()))
}

#[cfg(debug_assertions)]
static FAIL_RESERVE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_next_reservation() {
    FAIL_RESERVE.store(true, core::sync::atomic::Ordering::Release);
}

pub(crate) fn publish<T>(commit: impl FnOnce() -> (Arc<Task>, T)) -> Result<(Arc<Task>, T), ()> {
    let mut roster = roster_table().lock();
    #[cfg(debug_assertions)]
    if FAIL_RESERVE.swap(false, core::sync::atomic::Ordering::AcqRel) { return Err(()); }
    roster.try_reserve(1).map_err(|_| ())?;
    #[cfg(debug_assertions)]
    let _commit = crate::memory::allocator::NoAllocation::enter();
    let (task, value) = commit();
    let id = task.ident.id;
    let previous = roster.insert(id, TaskWeak::stored(Arc::downgrade(&task), Site::Roster));
    assert!(previous.is_none(), "publish: duplicate task id");
    Ok((task, value))
}

pub(crate) fn muster(id: TaskId) -> Option<TaskWeak> {
    roster_table()
        .lock()
        .get(&id)
        .map(|w| w.copy_at(Site::Muster))
}

pub(crate) fn prune_dead() -> usize {
    let mut g = roster_table().lock();
    let before = g.len();
    g.retain(|_, w| w.strong_count() > 0);
    before - g.len()
}

#[cfg(debug_assertions)]
pub(crate) fn roster_live_ids() -> (usize, [usize; 8]) {
    let g = roster_table().lock();
    let mut out = [0usize; 8];
    let mut n = 0usize;
    let mut more = 0usize;
    for (id, w) in g.iter() {
        if w.strong_count() == 0 {
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
    out.extend(g.values().map(|w| w.copy_at(Site::Snapshot)));
    out
}

pub(crate) fn remove_from_starved(target: &Arc<Task>) -> bool {
    for s in schedulers() {
        let mut i = s.inner.lock();
        if s.starved_remove(&mut i, target) {
            drop(i);
            return true;
        }
    }
    false
}

pub(crate) fn running_hart(target: &Arc<Task>) -> Option<HartId> {
    for s in schedulers() {
        let i = s.inner.lock();
        if i.running.as_ref().is_some_and(|t| Arc::ptr_eq(t, target)) {
            let h = s.hart;
            drop(i);
            return Some(h);
        }
    }
    None
}

pub(crate) fn current() -> &'static Scheduler {
    // SAFETY: tp 直达读出的指针非空
    unsafe { &*(crate::hart::scheduler() as *const Scheduler) }
}
