use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::TaskId;
use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::room::scheduler::core::muster;
use crate::work::unit::task::{Task, TaskState, TaskTag};
use crate::work::unit::team::Team;

use super::reap::reap;
use super::wait::holder::Ticket;
use super::wait::site::{SITE_SHARDS, WakeKey, shard_at};
use super::{prune, void};

pub(super) fn doomed() -> &'static SpinLock<HashMap<TaskId, usize>> {
    static T: OnceLock<SpinLock<HashMap<TaskId, usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

pub(super) static PENDING: AtomicUsize = AtomicUsize::new(0);

static CULL_HELD: AtomicUsize = AtomicUsize::new(0);
static CULL_STARVED: AtomicUsize = AtomicUsize::new(0);
static CULL_BLOCKED: AtomicUsize = AtomicUsize::new(0);
static NUDGED: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn branch_stats() -> (usize, usize, usize, usize) {
    (
        CULL_HELD.load(Ordering::Relaxed),
        CULL_STARVED.load(Ordering::Relaxed),
        CULL_BLOCKED.load(Ordering::Relaxed),
        NUDGED.load(Ordering::Relaxed),
    )
}

pub(super) fn rip() {
    doomed().lock().clear();
    PENDING.store(0, Ordering::Relaxed);
}

fn suspend(task: &Arc<Task>, reason: usize) -> bool {
    const RETRY: usize = 4;

    for _ in 0..=RETRY {
        let taken = match task.tag() {
            TaskTag::Reaped | TaskTag::Doomed => return false,
            TaskTag::Held => {
                let team = task.ident.team.clone();
                let ok = team.release_held(task);
                if ok {
                    CULL_HELD.fetch_add(1, Ordering::Relaxed);
                }
                ok
            }
            TaskTag::Starved => {
                let ok = crate::work::room::scheduler::core::remove_from_starved(task)
                    || task.boarding.lock().parked.take().is_some();
                if ok {
                    CULL_STARVED.fetch_add(1, Ordering::Relaxed);
                }
                ok
            }
            TaskTag::Blocked => {
                let Some(ticket) = pop_waiter(task) else {
                    continue;
                };
                void(ticket);
                CULL_BLOCKED.fetch_add(1, Ordering::Relaxed);
                true
            }
            TaskTag::Running => {
                let hart = crate::work::room::scheduler::core::running_hart(task);
                if hart.is_none() {
                    continue;
                }
                return doomed_nudge(task, reason);
            }
        };
        if taken {
            let mut t = task.clone();
            Task::exclusive(&mut t).transform(TaskState::Doomed);
            return true;
        }
    }
    doomed_nudge(task, reason)
}

fn pop_waiter(task: &Arc<Task>) -> Option<Ticket> {
    for shard in 0..SITE_SHARDS {
        let mut sites = shard_at(shard).lock();
        let mut hit: Option<(WakeKey, Ticket)> = None;
        for (key, site) in sites.iter_mut() {
            if let Some(ticket) = site
                .remove_if(&mut |t| Arc::ptr_eq(t, task))
                .map(|mut node| Task::blocked_ticket(&mut node))
            {
                hit = Some((*key, ticket));
                break;
            }
        }
        let out = hit.map(|(key, ticket)| {
            prune(&mut sites, key);
            ticket
        });
        drop(sites);
        if out.is_some() {
            return out;
        }
    }
    None
}

fn doomed_nudge(task: &Arc<Task>, reason: usize) -> bool {
    NUDGED.fetch_add(1, Ordering::Relaxed);
    let mut d = doomed().lock();
    if d.try_reserve(1).is_ok() && d.insert(task.ident.id, reason).is_none() {
        PENDING.fetch_add(1, Ordering::Relaxed);
    }
    drop(d);
    let hart = crate::work::room::scheduler::core::running_hart(task);
    if let Some(hart) = hart {
        crate::work::room::conductor::nudge(hart);
    }
    false
}

pub(crate) fn sweep_doomed() -> usize {
    const BATCH: usize = 4;

    if PENDING.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    let mut batch = [(TaskId::new(0), 0usize); BATCH];
    let mut n = 0;
    {
        let d = doomed().lock();
        for (tid, reason) in d.iter() {
            if n == BATCH {
                break;
            }
            batch[n] = (*tid, *reason);
            n += 1;
        }
    }
    let mut swept = 0;
    for &(tid, reason) in &batch[..n] {
        let Some(task) = muster(tid).and_then(|w| w.upgrade()) else {
            let _ = take_doomed(tid);
            continue;
        };
        if task.tag() == TaskTag::Reaped {
            let _ = take_doomed(tid);
            continue;
        }
        if suspend(&task, reason) {
            reap(task);
            let _ = take_doomed(tid);
            swept += 1;
        }
    }
    swept
}

pub(crate) fn cull(roots: &[Arc<Team>], reason: usize) {
    let mut work: Vec<Arc<Team>> = roots.to_vec();
    let mut tasks: Vec<Arc<Task>> = Vec::new();
    while let Some(t) = work.pop() {
        t.cancel_staging().expect("exit: cancel construction");
        for weak_task in t.tasks_snapshot() {
            if let Some(task) = weak_task.upgrade() {
                work.extend(task.heirs());
                tasks.push(task);
            }
        }
    }
    let victims: Vec<Arc<Task>> = tasks.into_iter().filter(|t| suspend(t, reason)).collect();
    for task in victims {
        reap(task);
    }
}

/// 退场钩子（连坐那一趟）：**照手上的这一具**，不再拿号回清册里找人。
pub(crate) fn doom(task: &Arc<Task>) {
    cull(&task.heirs(), super::EXIT_CASCADE);
}

pub(crate) fn take_doomed(tid: TaskId) -> Option<usize> {
    if PENDING.load(Ordering::Relaxed) == 0 {
        return None;
    }
    let out = doomed().lock().remove(&tid);
    if out.is_some() {
        PENDING.fetch_sub(1, Ordering::Relaxed);
    }
    out
}

#[allow(dead_code)]
pub(crate) fn descends(actor: &Arc<Team>, target: &Arc<Team>) -> bool {
    let mut team = target.clone();
    loop {
        let Some(sire) = team.sire().and_then(muster) else {
            return false;
        };
        let Some(parent) = Weak::upgrade(&sire) else {
            return false;
        };
        if Arc::ptr_eq(&parent.ident.team, actor) {
            return true;
        }
        team = parent.ident.team.clone();
    }
}

pub(crate) fn slay(task: &Arc<Task>) {
    if suspend(task, super::EXIT_DOOM) {
        reap(task.clone());
    }
}
