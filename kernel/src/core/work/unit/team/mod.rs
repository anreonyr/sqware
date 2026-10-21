use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{TaskId, TeamId, UnitFail};

use crate::core::memory::nest::NestError;
use crate::core::work::commit;
use crate::core::work::room::scheduler;
use crate::core::work::unit::life::TeamLife;
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::{Task, TaskBuilder, TaskState, TaskWeak};
use crate::machine::sync::{SpinLock, SpinLockGuard};

use lease::StagingLease;

mod builder;
mod construct;
pub(crate) mod exit;
mod kernel;
mod lease;
pub(super) mod life;
mod member;
mod oust;
mod phase;
mod receipt;
mod spawn;
mod stage;
mod staging;
mod state;
mod tasks;

pub(crate) use builder::TeamBuilder;
pub(crate) use kernel::{init_kernel, kernel};
pub(crate) use member::Member;
pub(crate) use oust::oust;
pub(crate) use phase::TeamPhase;
pub(crate) use receipt::Receipt;
pub(crate) use spawn::spawn;
pub(crate) use stage::TeamStage;
pub(crate) use staging::Staging;
pub(crate) use state::TeamState;
pub(crate) use tasks::Tasks;

pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_MEMBERS: usize = 1024;
pub(crate) const MAX_CHILDREN: usize = 1024;

/// 一个团队：地址空间、身份与家长，加状态机（`TeamState`）与成员表（`Tasks`）。
pub struct Team {
    pub(crate) space: Arc<Space>,
    pub(crate) id: TeamId,
    pub(crate) sire: TaskWeak,
    pub(crate) state: Arc<SpinLock<TeamState>>,
    pub(crate) tasks: Tasks,
    pub(crate) life: Arc<TeamLife>,
    operation: SpinLock<()>,
}

unsafe impl Sync for Team {}
unsafe impl Send for Team {}

impl Team {
    pub(crate) fn park(&self, task: Arc<Task>) {
        self.tasks.park(task);
    }

    pub(crate) fn debark(&self) -> Result<(), UnitFail> {
        let _commit = commit();
        self.state.lock().debark()?;
        let mut running = false;
        self.life.visit(&mut |node| {
            for member in node.tasks.lock().iter() {
                let hart = member.state.lock().on_hart();
                if let Some(hart) = hart {
                    running = true;
                    scheduler::idle::nudge(hart);
                }
            }
        });
        if running {
            return Err(UnitFail::Busy);
        }
        Ok(())
    }

    pub(crate) fn embark(&self) -> Result<(), UnitFail> {
        let _commit = commit();
        self.state.lock().embark()?;
        self.life.visit(&mut |node| {
            if node.paused() {
                return;
            }
            let mut at = 0;
            loop {
                let member = node.tasks.lock().get(at).cloned();
                let Some(member) = member else { break };
                at += 1;
                let Some(task) = member.task.upgrade() else {
                    continue;
                };
                if task.parked() {
                    *task.state.lock() = TaskState::ready();
                    task.ident.team.release_held(&task);
                    scheduler::launch(task);
                }
            }
        });
        Ok(())
    }

    pub(crate) fn paused(&self) -> bool {
        self.life.paused()
    }

    pub(crate) fn ready(&self) -> bool {
        self.state.lock().ready()
    }

    pub(crate) fn guard(&self, owner: TaskId) -> Result<(), UnitFail> {
        let _commit = commit();
        if !self.state.lock().debarked() || self.state.lock().constructing() {
            return Err(UnitFail::Denied);
        }
        if exit::conflicts(&self.life) || self.has_unavailable_members() {
            return Err(UnitFail::Busy);
        }
        exit::arm(&self.life, owner)
    }

    pub(crate) fn unguard(&self, owner: TaskId) -> Result<(), UnitFail> {
        let _commit = commit();
        if !self.state.lock().debarked() || self.state.lock().constructing() {
            return Err(UnitFail::Denied);
        }
        if self.has_unavailable_members() {
            return Err(UnitFail::Busy);
        }
        exit::disarm(&self.life, owner)
    }

    fn has_unavailable_members(&self) -> bool {
        let mut unavailable = false;
        self.life.visit(&mut |node| {
            for member in node.tasks.lock().iter() {
                let state = member.state.lock();
                if state.is_dead() || state.on_hart().is_some() {
                    unavailable = true;
                }
            }
        });
        unavailable
    }

    pub(crate) fn constructing(&self) -> bool {
        self.state.lock().constructing()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn staged_len(&self) -> usize {
        self.state.lock().staged_len()
    }

    /// 为暂存表留出容量。
    ///
    /// 扩容要向分配器取块，而状态锁是 `UnitState`（锁序里在分配器之后）：
    /// 所以把表摘下来在锁外扩容，再整表换回去；锁内只换表，不碰分配器。
    pub(crate) fn reserve_staging(&self, extra: usize) -> Result<(), NestError> {
        let mut staged = self.state.lock().take_staging();
        if staged.try_reserve(extra).is_err() {
            let shell = self.state.lock().swap_staging(staged);
            drop(shell);
            return Err(NestError::OutOfMemory);
        }
        let shell = self.state.lock().swap_staging(staged);
        drop(shell);
        Ok(())
    }

    pub(crate) fn push_staging(&self, item: Staging) {
        self.state.lock().push_staging(item);
    }

    pub(crate) fn find_staging(&self, hit: impl FnMut(&Staging) -> bool) -> Option<usize> {
        self.state.lock().find_staging(hit)
    }

    pub(crate) fn staged_at(&self, index: usize) -> Option<Staging> {
        self.state.lock().staged_at(index)
    }

    pub(crate) fn remove_staging(&self, index: usize) {
        self.state.lock().remove_staging(index);
    }

    fn take_staging(&self) -> Vec<Staging> {
        self.state.lock().take_staging()
    }

    /// 把租约摘走的暂存项还回团队。
    ///
    /// 合并与扩容同样在锁外做：锁内只换表。
    fn restore_staging(&self, mut items: Vec<Staging>) {
        let mut current = self.state.lock().take_staging();
        if !current.is_empty() {
            current.extend(items);
            items = current;
        }
        let shell = self.state.lock().swap_staging(items);
        drop(shell);
    }

    pub(crate) fn operation(&self) -> Option<SpinLockGuard<'_, ()>> {
        self.operation.try_lock()
    }

    pub(crate) fn cancel_staging(&self) -> Result<(), NestError> {
        let mut staged = StagingLease::hold(self);
        while let Some(item) = staged.items.last() {
            self.space.nest.retry_invalidation()?;
            let id = item.view.identity();
            if !self.space.nest.settled(id) {
                self.space.withdraw(item.view.whole())?;
            }
            self.space.retire_view(id);
            item.meta.detach_staging(item.token, self.id);
            let owner = item.meta.owner();
            let token = item.token;
            staged.items.pop();
            crate::core::work::pend::notify(owner, token);
        }
        Ok(())
    }

    pub(crate) fn retire(&self) -> Result<(), NestError> {
        let _operation = self.operation().ok_or(NestError::NoRegion)?;
        {
            let _commit = commit();
            if !self.all_reaped() {
                return Err(NestError::NoRegion);
            }
            self.life.visit(&mut |node| {
                let mut state = node.state.lock();
                if !state.terminal() {
                    state.phase = TeamPhase::Doomed;
                }
            });
        }
        let mut after = 0;
        while let Some(task) = scheduler::roster::after(after) {
            after = task.ident.id.get();
            if task.ident.team.id == self.id {
                if !task.reaped() {
                    return Err(NestError::NoRegion);
                }
                task.retire()?;
            }
        }
        self.cancel_staging()?;
        self.space.retire()
    }

    pub(crate) fn prune_tasks(&self) {
        let _commit = commit();
        self.life.prune();
    }

    pub(crate) fn all_reaped(&self) -> bool {
        self.life.all_reaped()
    }

    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }

    pub(crate) fn release_held(&self, task: &Arc<Task>) -> bool {
        self.tasks.release_held(task)
    }

    pub(crate) fn default_entry(&self) -> usize {
        self.state.lock().default_entry()
    }

    pub(crate) fn set_default_entry(&self, va: usize) {
        self.state.lock().set_default_entry(va);
    }

    pub(crate) fn sire(&self) -> Option<TaskId> {
        self.sire.upgrade().map(|t| t.ident.id)
    }

    /// 新任务的收据：从这棵树往上数，每个有主（`owner != 0`）的节点都算一条，
    /// 按家长号合并；深度超限或撞上已关的节点即拒绝。
    pub(crate) fn receipts(&self) -> Result<Vec<Receipt>, NestError> {
        let mut receipts: Vec<Receipt> = Vec::new();
        let mut node = Some(&*self.life);
        let mut depth = 0;
        while let Some(here) = node {
            depth += 1;
            if depth > MAX_DEPTH || here.closed() {
                return Err(NestError::NoRegion);
            }
            if here.owner.get() != 0 {
                if let Some(r) = receipts.iter_mut().find(|r| r.owner == here.owner) {
                    r.roots.try_reserve(1).map_err(|_| NestError::OutOfMemory)?;
                    r.roots.push(here.id);
                } else {
                    let mut roots = Vec::new();
                    roots.try_reserve(1).map_err(|_| NestError::OutOfMemory)?;
                    roots.push(here.id);
                    receipts
                        .try_reserve(1)
                        .map_err(|_| NestError::OutOfMemory)?;
                    receipts.push(Receipt {
                        owner: here.owner,
                        roots,
                    });
                }
            }
            node = here.parent.as_deref();
        }
        Ok(receipts)
    }
}

impl Drop for Team {
    fn drop(&mut self) {
        self.cancel_staging().expect("team staging release");
        let _commit = commit();
        self.state.lock().phase = TeamPhase::Ousted;
        self.life.prune();
        if let Some(parent) = &self.life.parent {
            parent.prune();
        }
    }
}
