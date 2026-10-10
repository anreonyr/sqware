use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};
use crate::lock::{Level, OnceLock, SpinLock};
use crate::lock::spin::SpinLockGuard;
use crate::work::unit::space::Space;
use super::life::Life;
use super::task::{Task, TaskBuilder, TaskExit, TaskState, TaskTag};
use super::weak::TaskWeak;

pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_MEMBERS: usize = 1024;
pub(crate) const MAX_CHILDREN: usize = 1024;

pub(crate) enum TeamState {
    Constructing { staged: Vec<Staging> },
    Ready { default_entry: usize },
    Debarking { state: TeamStage },
    Debarked { state: TeamStage },
    Doomed { staged: Vec<Staging> },
    Ousted,
}
// SAFETY: staging capability tokens are kernel records, accessed only under
// the TeamState lock and operation lease; no user handle is sent across harts.
unsafe impl Send for TeamState {}

pub(crate) enum TeamStage {
    Constructing { staged: Vec<Staging> },
    Ready { default_entry: usize },
}

pub(crate) struct Receipt { pub owner: TaskId, pub roots: Vec<TeamId> }
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
        match &*self.state.lock() {
            TaskState::Reaped { cause, reason } => Some(TaskExit {
                task: self.id, cause: *cause, reason: *reason,
            }),
            _ => None,
        }
    }
    pub fn pending(&self, owner: TaskId) -> bool {
        self.receipts.lock().iter().any(|r| r.owner == owner)
    }
    pub fn receive(&self, owner: TaskId) -> bool {
        let mut receipts = self.receipts.lock();
        if let Some(at) = receipts.iter().position(|r| r.owner == owner) {
            receipts.swap_remove(at); true
        } else { false }
    }
}

/// Resource-free topology. Reciprocal metadata edges are detached by prune,
/// after Ousted and the last receipt/descendant disappears; never retain Space.
pub(crate) struct TeamLife {
    pub id: TeamId,
    pub owner: TaskId,
    pub parent: Option<Arc<TeamLife>>,
    pub state: Arc<SpinLock<TeamState>>,
    pub tasks: Arc<SpinLock<Vec<Arc<Member>>>>,
    pub children: SpinLock<Vec<Arc<TeamLife>>>,
    pub life: Arc<Life>,
}
impl TeamLife {
    pub fn paused(&self) -> bool {
        let local = matches!(&*self.state.lock(), TeamState::Debarking { .. } | TeamState::Debarked { .. });
        local || self.parent.as_ref().is_some_and(|p| p.paused())
    }
    pub fn closed(&self) -> bool {
        let local = matches!(&*self.state.lock(), TeamState::Doomed { .. } | TeamState::Ousted);
        local || self.parent.as_ref().is_some_and(|p| p.closed())
    }
    pub fn member(&self, id: TaskId) -> Option<Arc<Member>> {
        self.tasks.lock().iter().find(|m| m.id == id).cloned()
    }
    pub fn visit(&self, run: &mut impl FnMut(&TeamLife)) {
        run(self);
        let mut after = 0;
        loop {
            let child = self.children.lock().iter().filter(|c| c.id.get() > after)
                .min_by_key(|c| c.id.get()).cloned();
            let Some(child) = child else { break };
            after = child.id.get(); child.visit(run);
        }
    }
    pub fn all_reaped(&self) -> bool {
        let mut done = true;
        self.visit(&mut |node| {
            if node.tasks.lock().iter().any(|m| m.exit().is_none()) { done = false; }
        });
        done
    }
    pub fn next(&self, owner: TaskId) -> Option<Arc<Member>> {
        let mut found = None;
        self.visit(&mut |node| {
            if found.is_none() {
                found = node.tasks.lock().iter().find(|m| m.pending(owner) && m.exit().is_some()).cloned();
            }
        });
        found
    }
    pub fn find(&self, id: TaskId) -> Option<Arc<Member>> {
        let mut found = None;
        self.visit(&mut |node| { if found.is_none() { found = node.member(id); } });
        found
    }
    pub fn clear(&self, owner: TaskId, root: Option<TeamId>) {
        self.visit(&mut |node| {
            for member in node.tasks.lock().iter() {
                let mut receipts = member.receipts.lock();
                for receipt in receipts.iter_mut().filter(|r| r.owner == owner) {
                    if let Some(root) = root { receipt.roots.retain(|id| *id != root); }
                    else { receipt.roots.clear(); }
                }
                receipts.retain(|r| !r.roots.is_empty());
            }
        });
    }
    pub fn prune(&self) {
        self.tasks.lock().retain(|m| m.exit().is_none() || !m.receipts.lock().is_empty());
        let mut at = 0;
        loop {
            let child = self.children.lock().get(at).cloned();
            let Some(child) = child else { break };
            child.prune();
            let terminal = matches!(&*child.state.lock(), TeamState::Ousted);
            let empty = child.tasks.lock().is_empty() && child.children.lock().is_empty();
            if terminal && empty { self.children.lock().remove(at); }
            else { at += 1; }
        }
    }
    pub fn doom(&self) {
        let _commit = super::commit();
        self.visit(&mut |node| {
            let mut state = node.state.lock();
            let old = core::mem::replace(&mut *state, TeamState::Ousted);
            *state = match old {
                TeamState::Constructing { staged }
                | TeamState::Debarking { state: TeamStage::Constructing { staged } }
                | TeamState::Debarked { state: TeamStage::Constructing { staged } } => TeamState::Doomed { staged },
                old @ (TeamState::Ousted | TeamState::Doomed { .. }) => old,
                _ => TeamState::Doomed { staged: Vec::new() },
            };
        });
    }
    pub fn notify(&self) {
        let mut node = Some(self);
        while let Some(here) = node {
            crate::work::room::messenger::signal(crate::work::room::messenger::WakeKey::Team { id: here.id });
            node = here.parent.as_deref();
        }
    }
}

/// Group the shared member records and execution-only keepalive index. TeamLife
/// clones only members, so retaining exit metadata never retains held Tasks.
pub(crate) struct Tasks {
    members: Arc<SpinLock<Vec<Arc<Member>>>>,
    pub(crate) held: SpinLock<Vec<Arc<Task>>>,
}
impl Deref for Tasks {
    type Target = SpinLock<Vec<Arc<Member>>>;
    fn deref(&self) -> &Self::Target { &self.members }
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

pub(crate) struct Staging {
    pub(crate) token: env::PieToken,
    pub(crate) meta: Arc<crate::work::mail::pole::PoleMeta>,
    pub(crate) span: super::space::Span,
}
pub(crate) struct Staged<'a>(SpinLockGuard<'a, TeamState>);
impl Deref for Staged<'_> {
    type Target = Vec<Staging>;
    fn deref(&self) -> &Self::Target { match &*self.0 {
        TeamState::Constructing { staged } | TeamState::Doomed { staged } => staged,
        TeamState::Debarking { state: TeamStage::Constructing { staged } }
        | TeamState::Debarked { state: TeamStage::Constructing { staged } } => staged,
        _ => panic!("staging outside construction"),
    } }
}
impl DerefMut for Staged<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target { match &mut *self.0 {
        TeamState::Constructing { staged } | TeamState::Doomed { staged } => staged,
        TeamState::Debarking { state: TeamStage::Constructing { staged } }
        | TeamState::Debarked { state: TeamStage::Constructing { staged } } => staged,
        _ => panic!("staging outside construction"),
    } }
}
struct StagingLease<'a> { team: &'a Team, items: Vec<Staging> }
impl Deref for StagingLease<'_> { type Target = Vec<Staging>; fn deref(&self) -> &Vec<Staging> { &self.items } }
impl DerefMut for StagingLease<'_> { fn deref_mut(&mut self) -> &mut Vec<Staging> { &mut self.items } }
impl Drop for StagingLease<'_> { fn drop(&mut self) { self.team.restore_staging(core::mem::take(&mut self.items)); } }

impl Team {
    pub(crate) fn park(&self, task: Arc<Task>) {
        let mut held = self.tasks.held.lock();
        if !held.iter().any(|t| Arc::ptr_eq(t, &task)) {
            assert!(held.len() < held.capacity(), "park capacity reserved before publish");
            held.push(task);
        }
    }
    pub(crate) fn debark(&self) -> Result<(), env::UnitFail> {
        let _commit = super::commit();
        {
            let mut state = self.state.lock();
            let old = core::mem::replace(&mut *state, TeamState::Ousted);
            *state = match old {
                TeamState::Constructing { staged } => TeamState::Debarking { state: TeamStage::Constructing { staged } },
                TeamState::Ready { default_entry } => TeamState::Debarking { state: TeamStage::Ready { default_entry } },
                old @ (TeamState::Debarking { .. } | TeamState::Debarked { .. }) => old,
                old => { *state = old; return Err(env::UnitFail::Denied); },
            };
        }
        let mut running = false;
        self.life.visit(&mut |node| {
            for member in node.tasks.lock().iter() {
                let hart = match &*member.state.lock() {
                    TaskState::Running { hart, .. } | TaskState::Debarking { hart, .. } => Some(*hart),
                    TaskState::Doomed { hart, .. } => *hart,
                    _ => None,
                };
                if let Some(hart) = hart { running = true; crate::work::room::conductor::nudge(hart); }
            }
        });
        if running { return Err(env::UnitFail::Busy); }
        let mut state = self.state.lock();
        let old = core::mem::replace(&mut *state, TeamState::Ousted);
        *state = match old { TeamState::Debarking { state } => TeamState::Debarked { state }, other => other };
        Ok(())
    }
    pub(crate) fn embark(&self) -> Result<(), env::UnitFail> {
        let _commit = super::commit();
        {
            let mut state = self.state.lock();
            let old = core::mem::replace(&mut *state, TeamState::Ousted);
            *state = match old {
                TeamState::Debarking { state } | TeamState::Debarked { state } => match state {
                    TeamStage::Constructing { staged } => TeamState::Constructing { staged },
                    TeamStage::Ready { default_entry } => TeamState::Ready { default_entry },
                },
                old @ (TeamState::Ready { .. } | TeamState::Constructing { .. }) => old,
                old => { *state = old; return Err(env::UnitFail::Denied); },
            };
        }
        self.life.visit(&mut |node| {
            if node.paused() { return; }
            let mut at = 0;
            loop {
                let member = node.tasks.lock().get(at).cloned();
                let Some(member) = member else { break }; at += 1;
                let Some(task) = member.task.upgrade() else { continue };
                if task.tag() == TaskTag::Parked {
                    *task.state.lock() = TaskState::Starved { next: None };
                    task.ident.team.release_held(&task);
                    crate::work::room::scheduler::core::launch(task);
                }
            }
        });
        Ok(())
    }
    pub(crate) fn paused(&self) -> bool { self.life.paused() }
    pub(crate) fn ready(&self) -> bool { matches!(&*self.state.lock(),
        TeamState::Ready { .. } | TeamState::Debarking { state: TeamStage::Ready { .. } }
        | TeamState::Debarked { state: TeamStage::Ready { .. } }) }
    pub(crate) fn staged(&self) -> Staged<'_> { Staged(self.state.lock()) }
    pub(crate) fn staged_len(&self) -> usize {
        match &*self.state.lock() {
            TeamState::Constructing { staged } | TeamState::Doomed { staged }
            | TeamState::Debarking { state: TeamStage::Constructing { staged } }
            | TeamState::Debarked { state: TeamStage::Constructing { staged } } => staged.len(),
            _ => 0,
        }
    }
    pub(crate) fn take_staging(&self) -> Vec<Staging> {
        if self.staged_len() == 0 { return Vec::new(); }
        core::mem::take(&mut *self.staged())
    }
    fn restore_staging(&self, items: Vec<Staging>) {
        if !items.is_empty() { self.staged().extend(items); }
    }
    pub(crate) fn operation(&self) -> Option<SpinLockGuard<'_, ()>> { self.operation.try_lock() }
    pub(crate) fn cancel_staging(&self) -> Result<(), crate::memory::manager::MapError> {
        let mut staged = StagingLease { team: self, items: self.take_staging() };
        while let Some(item) = staged.last() {
            self.space.release(item.span)?;
            item.meta.backing().unreserve();
            let owner = item.meta.owner(); let token = item.token;
            staged.pop();
            super::gate::notify(owner, token);
        }
        Ok(())
    }
    pub(crate) fn prune_tasks(&self, _exited: &Arc<Task>) {
        let _commit = super::commit(); self.life.prune();
    }
    pub(crate) fn all_reaped(&self) -> bool { self.life.all_reaped() }
    pub fn task(self: &Arc<Self>) -> TaskBuilder { TaskBuilder::new(self.clone()) }
    pub(crate) fn release_held(&self, task: &Arc<Task>) -> bool {
        let mut held = self.tasks.held.lock();
        if let Some(at) = held.iter().position(|t| Arc::ptr_eq(t, task)) { held.swap_remove(at); true }
        else { false }
    }
    pub(crate) fn default_entry(&self) -> usize {
        match &*self.state.lock() {
            TeamState::Ready { default_entry }
            | TeamState::Debarking { state: TeamStage::Ready { default_entry } }
            | TeamState::Debarked { state: TeamStage::Ready { default_entry } } => *default_entry,
            _ => 0,
        }
    }
    pub(crate) fn set_default_entry(&self, va: usize) { self.publish_entry(va); }
    fn publish_entry(&self, va: usize) {
        let mut state = self.state.lock();
        let next = match &*state {
            TeamState::Debarking { .. } => TeamState::Debarking { state: TeamStage::Ready { default_entry: va } },
            TeamState::Debarked { .. } => TeamState::Debarked { state: TeamStage::Ready { default_entry: va } },
            _ => TeamState::Ready { default_entry: va },
        };
        *state = next;
    }
    pub(crate) fn sire(&self) -> Option<TaskId> { self.sire.upgrade().map(|t| t.ident.id) }
    pub(crate) fn receipts(&self) -> Result<Vec<Receipt>, crate::memory::manager::MapError> {
        let mut receipts: Vec<Receipt> = Vec::new(); let mut node = Some(&*self.life); let mut depth = 0;
        while let Some(here) = node {
            depth += 1;
            if depth > MAX_DEPTH || here.closed() { return Err(crate::memory::manager::MapError::NoRegion); }
            if here.owner.get() != 0 {
                if let Some(r) = receipts.iter_mut().find(|r| r.owner == here.owner) {
                    r.roots.try_reserve(1).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
                    r.roots.push(here.id);
                } else {
                    let mut roots = Vec::new(); roots.try_reserve(1).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
                    roots.push(here.id);
                    receipts.try_reserve(1).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
                    receipts.push(Receipt { owner: here.owner, roots });
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
        *self.state.lock() = TeamState::Ousted;
        self.life.prune();
        if let Some(parent) = &self.life.parent { parent.prune(); }
    }
}

pub struct TeamBuilder { space: Space, sire: TaskWeak, constructing: bool }
impl TeamBuilder {
    pub fn new(space: Space) -> Self { Self { space, sire: TaskWeak::empty(), constructing: false } }
    pub fn sire(mut self, sire: TaskWeak) -> Self { self.sire = sire; self }
    pub(crate) fn constructing(mut self) -> Self { self.constructing = true; self }
    pub fn spawn(self) -> Result<Arc<Team>, crate::memory::manager::MapError> {
        let parent = self.sire.upgrade().map(|t| t.ident.team.life.clone());
        let owner = self.sire.upgrade().map_or(TaskId::new(0), |t| t.ident.id);
        let space = Arc::try_new(self.space).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
        let team = make_team(space, self.sire, parent, owner, self.constructing)?;
        if let Some(sire) = team.sire.upgrade() {
            sire.adopt(team.clone()).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
        }
        Ok(team)
    }
}
fn make_team(space: Arc<Space>, sire: TaskWeak, parent: Option<Arc<TeamLife>>, owner: TaskId, constructing: bool)
    -> Result<Arc<Team>, crate::memory::manager::MapError> {
    use crate::memory::manager::MapError;
    let id = alloc_team_id()?;
    let state = Arc::try_new(SpinLock::new_level(Level::UnitState, if constructing {
        TeamState::Constructing { staged: Vec::new() }
    } else { TeamState::Ready { default_entry: 0 } })).map_err(|_| MapError::OutOfMemory)?;
    let tasks = Arc::try_new(SpinLock::new_level(Level::TeamTasks, Vec::new())).map_err(|_| MapError::OutOfMemory)?;
    let life = Arc::try_new(TeamLife {
        id, owner, parent: parent.clone(), state: state.clone(), tasks: tasks.clone(),
        children: SpinLock::new(Vec::new()), life: Life::try_new().map_err(|_| MapError::OutOfMemory)?,
    }).map_err(|_| MapError::OutOfMemory)?;
    let team = Arc::try_new(Team { space, id, sire, state,
        tasks: Tasks { members: tasks, held: SpinLock::new_level(Level::L3, Vec::new()) },
        life: life.clone(), operation: SpinLock::new(()) })
        .map_err(|_| MapError::OutOfMemory)?;
    if let Some(parent) = parent {
        {
            let mut children = parent.children.lock();
            if children.len() >= MAX_CHILDREN { return Err(MapError::OutOfMemory); }
            children.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
        }
        let _commit = super::commit();
        let mut children = parent.children.lock();
        if parent.closed() { return Err(MapError::NoRegion); }
        let mut depth = 1;
        let mut ancestor = parent.parent.as_deref();
        while let Some(node) = ancestor { depth += 1; ancestor = node.parent.as_deref(); }
        if depth >= MAX_DEPTH { return Err(MapError::NoRegion); }
        if children.len() >= MAX_CHILDREN || children.len() == children.capacity() { return Err(MapError::OutOfMemory); }
        children.push(life);
    }
    Ok(team)
}
pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();
pub(crate) fn init_kernel(space: Arc<Space>) -> Result<&'static Arc<Team>, crate::memory::manager::MapError> {
    if KERNEL_TEAM.get().is_none() {
        let team = make_team(space, TaskWeak::empty(), None, TaskId::new(0), false)?;
        assert!(KERNEL_TEAM.set(team).is_ok());
    }
    Ok(KERNEL_TEAM.get().expect("kernel team initialized"))
}
pub fn kernel() -> Option<&'static Arc<Team>> { KERNEL_TEAM.get() }
static NEXT_TEAM_ID: AtomicUsize = AtomicUsize::new(1);
pub(crate) fn alloc_team_id() -> Result<TeamId, crate::memory::manager::MapError> {
    let id = NEXT_TEAM_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
        |id| (id < isize::MAX as usize).then_some(id + 1)).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
    Ok(TeamId::new(id))
}

/// Prepare and atomically publish one Held task, consuming private roots only at commit.
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
    // Every publication shares the operation lease with Oust, including an
    // already Ready team. Checking emptiness must exclude a late Spawn.
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
        use crate::work::unit::gate::{self, Permission};
        let mut prepared = builder.prepare().map_err(map_err)?;
        if !first {
            return prepared.publish(|| {}).map_err(map_err);
        }
        let caller = caller.ok_or(UnitFail::Denied)?;
        let mut staged = StagingLease { team: target, items: target.take_staging() };
        let mut operations = Vec::new();
        let mut retired = Vec::new();
        let mut leases = Vec::new();
        operations
            .try_reserve(staged.len())
            .map_err(|_| UnitFail::OoM)?;
        retired
            .try_reserve(staged.len())
            .map_err(|_| UnitFail::OoM)?;
        leases
            .try_reserve(staged.len())
            .map_err(|_| UnitFail::OoM)?;
        for item in staged.iter() {
            operations.push(item.meta.backing().operation().ok_or(UnitFail::Busy)?);
        }
        let closed = caller.gate.lock();
        if !super::gate::live(caller, ()) {
            return Err(UnitFail::Denied);
        }
        let mut pies = caller.gate.pies.lock();
        for item in staged.iter() {
            let Some(pie) = pies.iter().find(|p| p.token() == item.token) else {
                return Err(UnitFail::Denied);
            };
            let p = pie.snapshot().pole().ok_or(UnitFail::Denied)?;
            if !Arc::ptr_eq(&p, &item.meta)
                || !p.alive()
                || pie.sire().is_some()
                || pie.heir().is_some()
                || p.owner() != caller.ident.id
                || !pie
                    .permission()
                    .contains(Permission::FETCH | Permission::VEST | Permission::ONLY)
                || p.backing().reserved() != target.id.get()
                || !p.backing().unmapped()
                || p.mapped()
            {
                return Err(UnitFail::Denied);
            }
        }
        let result = prepared
            .publish(|| {
                for item in staged.drain(..) {
                    let index = pies
                        .iter()
                        .position(|p| p.token() == item.token)
                        .expect("staged root");
                    let pie = pies.remove(index);
                    pie.invalidate();
                    caller.gate.heirs.lock().retain(|(parent, _, _)| *parent != item.token);
                    retired.push(pie);
                    item.meta.backing().unreserve();
                    leases.push(item);
                }
                gate::changed(caller);
                target.set_default_entry(entry_va);
                target.publish_entry(entry_va);
            })
            .map_err(map_err);
        drop(pies);
        drop(closed);
        drop(staged);
        drop(operations);
        for pie in &retired { gate::notify(caller.ident.id, pie.token()); }
        drop(retired);
        drop(leases);
        result
    })();
    result
}
