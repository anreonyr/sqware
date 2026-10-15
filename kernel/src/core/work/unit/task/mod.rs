use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{TeamId, UnitFail};

use crate::core::work::commit;
use crate::core::work::pend::Pend;
use crate::core::work::room::messenger::{Ticket, WaitCall, WakeKey};
use crate::core::work::room::scheduler;
use crate::core::work::unit::life::Life;
use crate::core::work::unit::tally;
use crate::core::work::unit::team::{Team, TeamPhase};
use crate::machine::hart::HartId;
use crate::machine::sync::SpinLock;

use state::Step;

mod builder;
mod control;
mod ident;
pub(super) mod life;
mod prepared;
mod state;
mod weak;

pub(crate) use builder::TaskBuilder;
pub(crate) use ident::TaskIdent;
pub(crate) use state::TaskState;
pub(crate) use weak::TaskWeak;

#[cfg(debug_assertions)]
pub(crate) use builder::{fail_invalidation_at, fail_preparation_at, fail_rollback_fence};

pub use env::ExitCause as TaskExitCause;

pub(crate) const MAX_ARGS: usize = 64;

/// 一个任务：身份（`TaskIdent`，含它自己的内存段）、状态机（`TaskState`）与能力
/// 账本（`Pend`）的组合，加上它作为家长持有的子团队表。
pub struct Task {
    pub(crate) ident: Arc<TaskIdent>,
    pub(crate) life: Arc<Life>,
    pub(crate) state: Arc<SpinLock<TaskState>>,
    pub(crate) pend: Pend,
    pub(crate) heir: SpinLock<Vec<Arc<Team>>>,
    controls: SpinLock<Vec<(Weak<crate::core::work::unit::space::Space>, usize)>>,
}

unsafe impl Send for Task {}

impl Task {
    pub(crate) fn transform_checked(&self, next: TaskState) {
        self.state.lock().transform(next);
    }

    pub fn held(&self) -> bool {
        self.state.lock().is_held()
    }

    pub(crate) fn parked(&self) -> bool {
        self.state.lock().is_parked()
    }

    pub fn dead(&self) -> bool {
        self.state.lock().is_dead()
    }

    pub(crate) fn doomed(&self) -> bool {
        self.state.lock().is_doomed()
    }

    pub(crate) fn reaped(&self) -> bool {
        self.state.lock().is_reaped()
    }

    pub(crate) fn stopped(&self) -> bool {
        self.state.lock().paused()
    }

    pub(crate) fn on_hart(&self) -> Option<HartId> {
        self.state.lock().on_hart()
    }

    pub(crate) fn ticks_left(&self) -> u32 {
        self.state.lock().ticks_left()
    }

    pub(crate) fn doomed_on_hart(&self) -> Option<usize> {
        self.state.lock().doomed_on_hart()
    }

    pub(crate) fn block(&self, key: WakeKey, ticket: Ticket, wait: Option<WaitCall>) {
        self.state.lock().block(key, ticket, wait);
    }

    pub(crate) fn ready(&self) -> bool {
        self.state.lock().is_ready()
    }

    pub(crate) fn waiting(&self) -> bool {
        self.state.lock().is_waiting()
    }

    pub(crate) fn next(&self) -> Option<Arc<Task>> {
        self.state.lock().link().clone()
    }

    pub(crate) fn set_next(&self, value: Option<Arc<Task>>) {
        *self.state.lock().link() = value;
    }

    pub(crate) fn take_next(&self) -> Option<Arc<Task>> {
        self.state.lock().link().take()
    }

    pub(crate) fn wait_ticket(&self) -> Ticket {
        self.state.lock().wait_ticket()
    }

    pub(crate) fn take_wait(&self) -> Option<WaitCall> {
        self.state.lock().take_wait()
    }

    pub(crate) fn dec_ticks_left(&self) {
        self.state.lock().dec_ticks_left();
    }

    pub(crate) fn release(task: &Arc<Task>) -> Result<(), UnitFail> {
        let _commit = commit();
        if !task.held() || !task.ident.team.release_held(task) {
            return Err(UnitFail::Denied);
        }
        *task.state.lock() = TaskState::ready();
        scheduler::launch(task.clone());
        Ok(())
    }

    pub(crate) fn debark(task: &Arc<Task>) -> Result<(), UnitFail> {
        let _commit = commit();
        let step = task.state.lock().debark()?;
        match step {
            Step::Done => Ok(()),
            Step::LeaveReady => {
                if !scheduler::remove_from_starved(task) {
                    return Err(UnitFail::Busy);
                }
                task.state.lock().pause();
                task.ident.team.park(task.clone());
                Ok(())
            }
            Step::Nudge(hart) => {
                scheduler::idle::nudge(hart);
                Err(UnitFail::Busy)
            }
            other => unreachable!("debark: {other:?}"),
        }
    }

    pub(crate) fn embark(task: &Arc<Task>) -> Result<(), UnitFail> {
        let _commit = commit();
        if task.held() {
            return Self::release(task);
        }
        match task.state.lock().embark()? {
            Step::Done => Ok(()),
            Step::Held => Self::release(task),
            Step::Ready => {
                task.ident.team.release_held(task);
                scheduler::launch(task.clone());
                Ok(())
            }
            other => unreachable!("embark: {other:?}"),
        }
    }

    pub(crate) fn park(task: Arc<Task>) {
        let team = task.ident.team.clone();
        task.state.lock().park();
        team.park(task);
    }

    pub(crate) fn rise(task: &Arc<Task>) -> bool {
        let paused = task.state.lock().rise();
        if paused {
            task.ident.team.park(task.clone());
            false
        } else {
            true
        }
    }

    fn count_vanished(&self) {
        if !self.reaped() {
            tally::exit();
        }
    }

    pub(crate) fn adopt(&self, child: Arc<Team>) -> Result<(), ()> {
        {
            self.heir.lock().try_reserve(1).map_err(|_| ())?;
        }
        let _commit = commit();
        if self.dead() {
            return Err(());
        }
        let mut g = self.heir.lock();
        if g.len() == g.capacity() {
            return Err(());
        }
        g.push(child);
        Ok(())
    }

    pub(crate) fn oust(&self, team: TeamId) -> Option<Arc<Team>> {
        let _commit = commit();
        let child = {
            let mut heirs = self.heir.lock();
            let at = heirs.iter().position(|t| t.id == team)?;
            heirs.remove(at)
        };
        child.state.lock().phase = TeamPhase::Ousted;
        child.life.clear(self.ident.id, Some(team));
        let mut root = child.life.clone();
        while let Some(parent) = &root.parent {
            root = parent.clone();
        }
        root.prune();
        Some(child)
    }

    pub(crate) fn heir_node(&self, at: usize) -> Option<Arc<Team>> {
        self.heir.lock().get(at).cloned()
    }

    pub(crate) fn heir(&self, id: TeamId) -> Option<Arc<Team>> {
        self.heir.lock().iter().find(|t| t.id == id).cloned()
    }

    pub(crate) fn clan(&self, id: TeamId) -> Option<Arc<Team>> {
        let _commit = commit();
        if id == self.ident.team.id {
            return None;
        }
        if let Some(child) = self.heir(id) {
            return Some(child);
        }
        let mut found = None;
        self.ident.team.life.visit(&mut |node| {
            if node.id != id {
                return;
            }
            found = node
                .parent
                .as_ref()
                .and_then(|parent| parent.member(node.owner))
                .and_then(|member| member.task.upgrade())
                .and_then(|sire| sire.heir(id))
                .or_else(|| {
                    let mut at = 0;
                    loop {
                        let member = node.tasks.lock().get(at).cloned()?;
                        at += 1;
                        if let Some(task) = member.task.upgrade() {
                            return Some(task.ident.team.clone());
                        }
                    }
                });
        });
        found.filter(|team| !team.state.lock().phase.ousted())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn heir_count(&self) -> usize {
        self.heir.lock().len()
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let _commit = commit();
        for child in self.heir.lock().iter() {
            child.life.clear(self.ident.id, None);
            child.life.prune();
        }
        for pie in self.pend.pies.lock().iter() {
            pie.invalidate();
        }
        self.count_vanished();
    }
}
