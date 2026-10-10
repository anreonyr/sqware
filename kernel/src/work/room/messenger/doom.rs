use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use env::TaskId;
use crate::work::room::scheduler::core::muster;
use crate::work::unit::task::{Task, TaskExitCause, TaskState, TaskTag, TaskStopped};
use crate::work::unit::team::{Team, TeamLife};
use super::reap::reap;
use super::wait::holder::Ticket;
use super::wait::site::{SITE_SHARDS, WakeKey, shard_at};
use super::{prune, void};
static CULL_HELD: AtomicUsize = AtomicUsize::new(0);
static CULL_STARVED: AtomicUsize = AtomicUsize::new(0);
static CULL_BLOCKED: AtomicUsize = AtomicUsize::new(0);
static NUDGED: AtomicUsize = AtomicUsize::new(0);
pub(crate) fn branch_stats() -> (usize, usize, usize, usize) {
    (CULL_HELD.load(Ordering::Relaxed), CULL_STARVED.load(Ordering::Relaxed),
     CULL_BLOCKED.load(Ordering::Relaxed), NUDGED.load(Ordering::Relaxed))
}
pub(super) fn rip() {}

fn suspend(task: &Arc<Task>, cause: TaskExitCause, reason: usize) -> bool {
    let _commit = crate::work::unit::commit();
    match task.tag() {
        TaskTag::Doomed | TaskTag::Reaped => return false,
        TaskTag::Running | TaskTag::Debarking => {
            let hart = match &*task.state.lock() {
                TaskState::Running { hart, .. } | TaskState::Debarking { hart, .. } => *hart,
                _ => unreachable!(),
            };
            *task.state.lock() = TaskState::Doomed { hart: Some(hart), cause, reason };
            NUDGED.fetch_add(1, Ordering::Relaxed);
            crate::work::room::conductor::nudge(hart); return false;
        }
        TaskTag::Starved => {
            assert!(crate::work::room::scheduler::core::remove_from_starved(task));
            CULL_STARVED.fetch_add(1, Ordering::Relaxed);
        }
        TaskTag::Blocked => {
            let ticket = pop_waiter(task).expect("published blocked waiter"); void(ticket);
            CULL_BLOCKED.fetch_add(1, Ordering::Relaxed);
        }
        TaskTag::Debarked => {
            let blocked = matches!(&*task.state.lock(), TaskState::Debarked { state: TaskStopped::Blocked { .. } });
            if blocked { let ticket = pop_waiter(task).expect("stopped blocked waiter"); void(ticket); }
            else { task.ident.team.release_held(task); }
        }
        TaskTag::Held | TaskTag::Parked => {
            assert!(task.ident.team.release_held(task));
            CULL_HELD.fetch_add(1, Ordering::Relaxed);
        }
    }
    *task.state.lock() = TaskState::Doomed { hart: None, cause, reason };
    true
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


fn cull_life(root: &TeamLife, cause: TaskExitCause, reason: usize) {
    root.doom();
    root.visit(&mut |node| {
        let mut after = 0;
        loop {
            let member = node.tasks.lock().iter().filter(|m| m.id.get() > after)
                .min_by_key(|m| m.id.get()).cloned();
            let Some(member) = member else { break }; after = member.id.get();
            if let Some(task) = member.task.upgrade() {
                if suspend(&task, cause, reason) { reap(task, cause, reason); }
            }
        }
    });
}
pub(crate) fn cull(roots: &[Arc<Team>], reason: usize) {
    for root in roots { root.cancel_staging().expect("cancel construction"); cull_life(&root.life, TaskExitCause::Slay, reason); }
}
pub(crate) fn doom(task: &Arc<Task>) {
    let mut at = 0;
    while let Some(child) = task.heir_node(at) {
        at += 1;
        { let _commit = crate::work::unit::commit(); child.life.clear(task.ident.id, None); }
        child.cancel_staging().expect("cancel construction");
        cull_life(&child.life, TaskExitCause::Cascade, super::EXIT_CASCADE);
    }
}
/// Running termination is stored only in TaskState; the hart checks it before
/// returning to userspace. No allocating request map or mirrored reason.
pub(crate) fn take_doomed(tid: TaskId) -> Option<usize> {
    let task = muster(tid)?.upgrade()?;
    match &*task.state.lock() { TaskState::Doomed { hart: Some(_), reason, .. } => Some(*reason), _ => None }
}
pub(crate) fn sweep_doomed() -> usize { 0 }
pub(crate) fn slay(task: &Arc<Task>) {
    if suspend(task, TaskExitCause::Slay, super::EXIT_DOOM) { reap(task.clone(), TaskExitCause::Slay, super::EXIT_DOOM); }
}
