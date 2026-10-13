use super::reap::reap;
use super::wait::holder::Ticket;
use super::wait::site::{SITE_SHARDS, WakeKey, shard_at};
use super::{prune, void};
use crate::work::room::Counter;
use crate::work::room::scheduler::core::muster;
use crate::work::unit::life::TeamLife;
use crate::work::unit::task::{Task, TaskExitCause};
use crate::work::unit::team::Team;
use alloc::sync::Arc;
use env::TaskId;
static CULL_HELD: Counter = Counter::new();
static CULL_STARVED: Counter = Counter::new();
static CULL_BLOCKED: Counter = Counter::new();
static NUDGED: Counter = Counter::new();
pub(crate) fn branch_stats() -> (usize, usize, usize, usize) {
    (
        CULL_HELD.get(),
        CULL_STARVED.get(),
        CULL_BLOCKED.get(),
        NUDGED.get(),
    )
}
fn suspend(task: &Arc<Task>, cause: TaskExitCause, reason: usize) -> bool {
    let _commit = crate::work::unit::commit();
    if task.dead() {
        return false;
    }
    if let Some(hart) = task.on_hart() {
        task.state.lock().doom(Some(hart), cause, reason);
        NUDGED.bump();
        crate::work::room::conductor::nudge(hart);
        return false;
    }
    if task.ready() && !task.stopped() {
        assert!(crate::work::room::scheduler::core::remove_from_starved(
            task
        ));
        CULL_STARVED.bump();
    } else if task.waiting() {
        let ticket = pop_waiter(task).expect("published blocked waiter");
        void(ticket);
        CULL_BLOCKED.bump();
    } else {
        assert!(task.ident.team.release_held(task));
        CULL_HELD.bump();
    }
    if let Some(super::WaitCall::Mail(wait)) = task.take_wait() {
        wait.unwatch();
    }
    task.state.lock().doom(None, cause, reason);
    true
}
fn pop_waiter(task: &Arc<Task>) -> Option<Ticket> {
    for shard in 0..SITE_SHARDS {
        let mut sites = shard_at(shard).lock();
        let mut hit: Option<(WakeKey, Ticket)> = None;
        for (key, site) in sites.iter_mut() {
            if let Some(ticket) = site
                .chain
                .remove_if(&mut |t| Arc::ptr_eq(t, task))
                .map(|node| node.wait_ticket())
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
            let member = node
                .tasks
                .lock()
                .iter()
                .filter(|m| m.id.get() > after)
                .min_by_key(|m| m.id.get())
                .cloned();
            let Some(member) = member else { break };
            after = member.id.get();
            if let Some(task) = member.task.upgrade() {
                if suspend(&task, cause, reason) {
                    reap(task, cause, reason);
                }
            }
        }
    });
}
pub(crate) fn cull(roots: &[Arc<Team>], reason: usize) {
    for root in roots {
        root.cancel_staging().expect("cancel construction");
        cull_life(&root.life, TaskExitCause::Slay, reason);
    }
}
pub(crate) fn doom(task: &Arc<Task>) {
    let mut at = 0;
    while let Some(child) = task.heir_node(at) {
        at += 1;
        {
            let _commit = crate::work::unit::commit();
            child.life.clear(task.ident.id, None);
        }
        child.cancel_staging().expect("cancel construction");
        cull_life(&child.life, TaskExitCause::Cascade, super::EXIT_CASCADE);
    }
}
pub(crate) fn take_doomed(tid: TaskId) -> Option<usize> {
    let task = muster(tid)?.upgrade()?;
    task.doomed_on_hart()
}
pub(crate) fn slay(task: &Arc<Task>) {
    if suspend(task, TaskExitCause::Slay, super::EXIT_DOOM) {
        reap(task.clone(), TaskExitCause::Slay, super::EXIT_DOOM);
    }
}
