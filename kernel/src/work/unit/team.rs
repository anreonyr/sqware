use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::life::{Life, TeamLife};
use super::task::{Task, TaskBuilder, TaskExit, TaskState, TaskWeak};
use crate::lock::spin::SpinLockGuard;
use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::space::Space;
use env::{TaskId, TeamId};

pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_MEMBERS: usize = 1024;
pub(crate) const MAX_CHILDREN: usize = 1024;

pub(crate) struct TeamState {
    pub(crate) stage: TeamStage,
    pub(crate) phase: TeamPhase,
    pub(crate) staged: Vec<Staging>,
}
unsafe impl Send for TeamState {}

impl TeamState {
    fn new(stage: TeamStage) -> TeamState {
        TeamState {
            stage,
            phase: TeamPhase::Live,
            staged: Vec::new(),
        }
    }
}

pub(crate) enum TeamStage {
    Constructing,
    Ready { default_entry: usize },
}

pub(crate) enum TeamPhase {
    Live,
    Paused,
    Doomed,
    Ousted,
}

pub(crate) struct Receipt {
    pub owner: TaskId,
    pub roots: Vec<TeamId>,
}
pub(crate) struct Member {
    pub id: TaskId,
    pub task: TaskWeak,
    pub node: alloc::sync::Weak<TeamLife>,
    pub state: Arc<SpinLock<TaskState>>,
    pub life: Arc<Life>,
    pub receipts: SpinLock<Vec<Receipt>>,
}
impl Member {
    pub fn exit(&self) -> Option<TaskExit> {
        let (cause, reason) = self.state.lock().exit()?;
        Some(TaskExit {
            task: self.id,
            cause,
            reason,
        })
    }
    pub fn pending(&self, owner: TaskId) -> bool {
        self.receipts.lock().iter().any(|r| r.owner == owner)
    }
    pub fn receive(&self, owner: TaskId) -> bool {
        let mut receipts = self.receipts.lock();
        if let Some(at) = receipts.iter().position(|r| r.owner == owner) {
            receipts.swap_remove(at);
            true
        } else {
            false
        }
    }
}

pub(crate) struct Tasks {
    pub(crate) members: Arc<SpinLock<Vec<Arc<Member>>>>,
    pub(crate) held: SpinLock<Vec<Arc<Task>>>,
}
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

#[derive(Clone)]
pub(crate) struct Staging {
    pub(crate) token: env::PieToken,
    pub(crate) meta: Arc<crate::work::mail::pole::PoleMeta>,
    pub(crate) map: Arc<crate::memory::manager::Map>,
}
struct StagingLease<'a> {
    team: &'a Team,
    items: Vec<Staging>,
}
impl Drop for StagingLease<'_> {
    fn drop(&mut self) {
        self.team.restore_staging(core::mem::take(&mut self.items));
    }
}

impl Team {
    pub(crate) fn park(&self, task: Arc<Task>) {
        let mut held = self.tasks.held.lock();
        if !held.iter().any(|t| Arc::ptr_eq(t, &task)) {
            assert!(
                held.len() < held.capacity(),
                "park capacity reserved before publish"
            );
            held.push(task);
        }
    }
    pub(crate) fn debark(&self) -> Result<(), env::UnitFail> {
        let _commit = super::commit();
        {
            let mut state = self.state.lock();
            match state.phase {
                TeamPhase::Live => state.phase = TeamPhase::Paused,
                TeamPhase::Paused => {}
                TeamPhase::Doomed | TeamPhase::Ousted => return Err(env::UnitFail::Denied),
            }
        }
        let mut running = false;
        self.life.visit(&mut |node| {
            for member in node.tasks.lock().iter() {
                let hart = member.state.lock().on_hart();
                if let Some(hart) = hart {
                    running = true;
                    crate::work::room::conductor::nudge(hart);
                }
            }
        });
        if running {
            return Err(env::UnitFail::Busy);
        }
        Ok(())
    }
    pub(crate) fn embark(&self) -> Result<(), env::UnitFail> {
        let _commit = super::commit();
        {
            let mut state = self.state.lock();
            match state.phase {
                TeamPhase::Paused => state.phase = TeamPhase::Live,
                TeamPhase::Live => {}
                TeamPhase::Doomed | TeamPhase::Ousted => return Err(env::UnitFail::Denied),
            }
        }
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
                    crate::work::room::scheduler::core::launch(task);
                }
            }
        });
        Ok(())
    }
    pub(crate) fn paused(&self) -> bool {
        self.life.paused()
    }
    pub(crate) fn ready(&self) -> bool {
        let state = self.state.lock();
        matches!(state.stage, TeamStage::Ready { .. }) && !Self::terminal(&state)
    }
    pub(crate) fn constructing(&self) -> bool {
        let state = self.state.lock();
        matches!(state.stage, TeamStage::Constructing) && !Self::terminal(&state)
    }
    fn terminal(state: &TeamState) -> bool {
        matches!(state.phase, TeamPhase::Doomed | TeamPhase::Ousted)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn staged_len(&self) -> usize {
        self.state.lock().staged.len()
    }
    pub(crate) fn reserve_staging(
        &self,
        extra: usize,
    ) -> Result<(), crate::memory::manager::MapError> {
        self.state
            .lock()
            .staged
            .try_reserve(extra)
            .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
    }
    pub(crate) fn push_staging(&self, item: Staging) {
        self.state.lock().staged.push(item);
    }
    pub(crate) fn find_staging(&self, hit: impl FnMut(&Staging) -> bool) -> Option<usize> {
        self.state.lock().staged.iter().position(hit)
    }
    pub(crate) fn staged_at(&self, index: usize) -> Option<Staging> {
        self.state.lock().staged.get(index).cloned()
    }
    pub(crate) fn remove_staging(&self, index: usize) {
        let mut state = self.state.lock();
        if index < state.staged.len() {
            state.staged.remove(index);
        }
    }
    pub(crate) fn take_staging(&self) -> Vec<Staging> {
        core::mem::take(&mut self.state.lock().staged)
    }
    fn restore_staging(&self, items: Vec<Staging>) {
        if !items.is_empty() {
            self.state.lock().staged.extend(items);
        }
    }
    pub(crate) fn operation(&self) -> Option<SpinLockGuard<'_, ()>> {
        self.operation.try_lock()
    }
    pub(crate) fn cancel_staging(&self) -> Result<(), crate::memory::manager::MapError> {
        let mut staged = StagingLease {
            team: self,
            items: self.take_staging(),
        };
        while let Some(item) = staged.items.last() {
            let (start, size) = item.map.initial_range();
            self.space.unmap_range(start, size)?;
            item.meta.detach_staging(item.token, self.id);
            let owner = item.meta.owner();
            let token = item.token;
            staged.items.pop();
            crate::work::pend::notify(owner, token);
        }
        Ok(())
    }
    pub(crate) fn prune_tasks(&self) {
        let _commit = super::commit();
        self.life.prune();
    }
    pub(crate) fn all_reaped(&self) -> bool {
        self.life.all_reaped()
    }
    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }
    pub(crate) fn release_held(&self, task: &Arc<Task>) -> bool {
        let mut held = self.tasks.held.lock();
        if let Some(at) = held.iter().position(|t| Arc::ptr_eq(t, task)) {
            held.swap_remove(at);
            true
        } else {
            false
        }
    }
    pub(crate) fn default_entry(&self) -> usize {
        let state = self.state.lock();
        match (&state.stage, Self::terminal(&state)) {
            (TeamStage::Ready { default_entry }, false) => *default_entry,
            _ => 0,
        }
    }
    pub(crate) fn set_default_entry(&self, va: usize) {
        self.state.lock().stage = TeamStage::Ready { default_entry: va };
    }
    pub(crate) fn sire(&self) -> Option<TaskId> {
        self.sire.upgrade().map(|t| t.ident.id)
    }
    pub(crate) fn receipts(&self) -> Result<Vec<Receipt>, crate::memory::manager::MapError> {
        let mut receipts: Vec<Receipt> = Vec::new();
        let mut node = Some(&*self.life);
        let mut depth = 0;
        while let Some(here) = node {
            depth += 1;
            if depth > MAX_DEPTH || here.closed() {
                return Err(crate::memory::manager::MapError::NoRegion);
            }
            if here.owner.get() != 0 {
                if let Some(r) = receipts.iter_mut().find(|r| r.owner == here.owner) {
                    r.roots
                        .try_reserve(1)
                        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
                    r.roots.push(here.id);
                } else {
                    let mut roots = Vec::new();
                    roots
                        .try_reserve(1)
                        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
                    roots.push(here.id);
                    receipts
                        .try_reserve(1)
                        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
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
        let _commit = super::commit();
        self.state.lock().phase = TeamPhase::Ousted;
        self.life.prune();
        if let Some(parent) = &self.life.parent {
            parent.prune();
        }
    }
}

pub struct TeamBuilder {
    space: Space,
    sire: TaskWeak,
    constructing: bool,
}
impl TeamBuilder {
    pub fn new(space: Space) -> Self {
        Self {
            space,
            sire: TaskWeak::empty(),
            constructing: false,
        }
    }
    pub fn sire(mut self, sire: TaskWeak) -> Self {
        self.sire = sire;
        self
    }
    pub(crate) fn constructing(mut self) -> Self {
        self.constructing = true;
        self
    }
    pub fn spawn(self) -> Result<Arc<Team>, crate::memory::manager::MapError> {
        let parent = self.sire.upgrade().map(|t| t.ident.team.life.clone());
        let owner = self.sire.upgrade().map_or(TaskId::new(0), |t| t.ident.id);
        let space =
            Arc::try_new(self.space).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
        let team = make_team(space, self.sire, parent, owner, self.constructing)?;
        if let Some(sire) = team.sire.upgrade() {
            if sire.adopt(team.clone()).is_err() {
                team.state.lock().phase = TeamPhase::Ousted;
                if let Some(parent) = &team.life.parent {
                    parent
                        .children
                        .lock()
                        .retain(|node| !Arc::ptr_eq(node, &team.life));
                }
                return Err(if sire.dead() {
                    crate::memory::manager::MapError::NoRegion
                } else {
                    crate::memory::manager::MapError::OutOfMemory
                });
            }
        }
        Ok(team)
    }
}
fn make_team(
    space: Arc<Space>,
    sire: TaskWeak,
    parent: Option<Arc<TeamLife>>,
    owner: TaskId,
    constructing: bool,
) -> Result<Arc<Team>, crate::memory::manager::MapError> {
    use crate::memory::manager::MapError;
    let id = next_id()?;
    let state = Arc::try_new(SpinLock::new_level(
        Level::UnitState,
        TeamState::new(if constructing {
            TeamStage::Constructing
        } else {
            TeamStage::Ready { default_entry: 0 }
        }),
    ))
    .map_err(|_| MapError::OutOfMemory)?;
    let tasks = Arc::try_new(SpinLock::new_level(Level::TeamTasks, Vec::new()))
        .map_err(|_| MapError::OutOfMemory)?;
    let life = Arc::try_new(TeamLife {
        id,
        owner,
        parent: parent.clone(),
        state: state.clone(),
        tasks: tasks.clone(),
        children: SpinLock::new(Vec::new()),
        life: Life::try_new().map_err(|_| MapError::OutOfMemory)?,
    })
    .map_err(|_| MapError::OutOfMemory)?;
    let team = Arc::try_new(Team {
        space,
        id,
        sire,
        state,
        tasks: Tasks {
            members: tasks,
            held: SpinLock::new_level(Level::L3, Vec::new()),
        },
        life: life.clone(),
        operation: SpinLock::new(()),
    })
    .map_err(|_| MapError::OutOfMemory)?;
    if let Some(parent) = parent {
        {
            let mut children = parent.children.lock();
            if children.len() >= MAX_CHILDREN {
                return Err(MapError::OutOfMemory);
            }
            children.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
        }
        let _commit = super::commit();
        let mut children = parent.children.lock();
        if parent.closed() {
            return Err(MapError::NoRegion);
        }
        let mut depth = 1;
        let mut ancestor = parent.parent.as_deref();
        while let Some(node) = ancestor {
            depth += 1;
            ancestor = node.parent.as_deref();
        }
        if depth >= MAX_DEPTH {
            return Err(MapError::NoRegion);
        }
        if children.len() >= MAX_CHILDREN || children.len() == children.capacity() {
            return Err(MapError::OutOfMemory);
        }
        children.push(life);
    }
    Ok(team)
}
pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();
pub(crate) fn init_kernel(
    space: Arc<Space>,
) -> Result<&'static Arc<Team>, crate::memory::manager::MapError> {
    if KERNEL_TEAM.get().is_none() {
        let team = make_team(space, TaskWeak::empty(), None, TaskId::new(0), false)?;
        assert!(KERNEL_TEAM.set(team).is_ok());
    }
    Ok(KERNEL_TEAM.get().expect("kernel team initialized"))
}
pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}
static NEXT: AtomicUsize = AtomicUsize::new(1);
pub(crate) fn next_id() -> Result<TeamId, crate::memory::manager::MapError> {
    let id = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            (id < isize::MAX as usize).then_some(id + 1)
        })
        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
    Ok(TeamId::new(id))
}

pub(crate) fn spawn(
    target: &Arc<Team>,
    caller: Option<&Arc<Task>>,
    entry: usize,
    words: Vec<usize>,
    stack: usize,
) -> Result<Arc<Task>, env::UnitFail> {
    use crate::memory::manager::{MapError, addr::VirtAddr as KVirt};
    use env::UnitFail;
    fn map_err(error: MapError) -> UnitFail {
        if error == MapError::OutOfMemory {
            UnitFail::OoM
        } else {
            UnitFail::Denied
        }
    }
    let _construction = target.operation().ok_or(UnitFail::Busy)?;
    let first = !target.ready();
    let entry_va = if entry == 0 {
        target.default_entry()
    } else {
        entry
    };
    if !entry_va.is_multiple_of(2)
        || !Space::user_range(entry_va, 2)
        || target.space.instruction_byte(entry_va).is_none()
        || target.space.instruction_byte(entry_va + 1).is_none()
    {
        return Err(env::UnitFail::BadEntry);
    }
    let mut builder = target.task().entry(KVirt::from_raw(entry_va)).args(words);
    if stack > 0 {
        builder = builder.stack(stack);
    }
    let result = (|| -> Result<Arc<Task>, UnitFail> {
        use crate::work::pend::{self, Permission};
        let mut prepared = builder.prepare().map_err(map_err)?;
        if !first {
            return prepared.publish(|| {}).map_err(map_err);
        }
        let caller = caller.ok_or(UnitFail::Denied)?;
        let mut staged = StagingLease {
            team: target,
            items: target.take_staging(),
        };
        let mut operations = Vec::new();
        let mut retired = Vec::new();
        let mut leases = Vec::new();
        operations
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        retired
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        leases
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        for item in staged.items.iter() {
            operations.push(item.meta.operation().ok_or(UnitFail::Busy)?);
        }
        let closed = caller.pend.lock();
        if !crate::work::pend::live(caller) {
            return Err(UnitFail::Denied);
        }
        let mut pies = caller.pend.pies.lock();
        for item in staged.items.iter() {
            let Some(pie) = pies.iter().find(|p| p.token() == item.token) else {
                return Err(UnitFail::Denied);
            };
            let p = pie.snapshot().mail().pole().ok_or(UnitFail::Denied)?;
            if !Arc::ptr_eq(&p, &item.meta)
                || !p.alive()
                || pie.sire().is_some()
                || pie.heir().is_some()
                || p.owner() != caller.ident.id
                || !pie
                    .permission()
                    .contains(Permission::FETCH | Permission::VEST | Permission::ONLY)
                || p.reserved() != target.id.get()
                || p.mapped()
            {
                return Err(UnitFail::Denied);
            }
        }
        let result = prepared
            .publish(|| {
                for item in staged.items.drain(..) {
                    let index = pies
                        .iter()
                        .position(|p| p.token() == item.token)
                        .expect("staged root");
                    let pie = pies.remove(index);
                    pie.invalidate();
                    caller
                        .pend
                        .heirs
                        .lock()
                        .retain(|(parent, _, _)| *parent != item.token);
                    retired.push(pie);
                    item.meta.detach_staging(item.token, target.id);
                    leases.push(item);
                }
                pend::changed(caller);
                target.set_default_entry(entry_va);
            })
            .map_err(map_err);
        drop(pies);
        drop(closed);
        drop(staged);
        drop(operations);
        for pie in &retired {
            pend::notify(caller.ident.id, pie.token());
        }
        drop(retired);
        drop(leases);
        result
    })();
    result
}
