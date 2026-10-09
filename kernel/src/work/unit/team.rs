use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::space::Space;

use super::task::{Task, TaskBuilder, TaskTag};
use super::weak::{Site, TaskWeak};

struct Completion { task: TaskId, reason: Option<env::Reason>, fault: Option<env::Reason> }

pub struct Team {
    pub(crate) space: Arc<Space>,
    pub(crate) tasks: SpinLock<Vec<TaskWeak>>,
    pub(crate) held: SpinLock<Vec<Arc<Task>>>,
    pub(crate) id: TeamId,
    pub(crate) sire: TaskWeak,
    default_entry: OnceLock<usize>,
    pub(crate) ready: AtomicBool,
    operating: AtomicBool,
    paused: AtomicBool,
    revision: AtomicUsize,
    completion: SpinLock<Option<Completion>>,
    observed: AtomicBool,
    observer: Option<Weak<Team>>,
    pub(crate) staged: SpinLock<Vec<Staging>>,
}

// SAFETY: shared team state is protected by atomics, OnceLock, and container locks.
unsafe impl Sync for Team {}
// SAFETY: the owned Space and synchronized task/resource references can cross harts.
unsafe impl Send for Team {}

pub(crate) struct Staging {
    pub(crate) token: env::PieToken,
    pub(crate) meta: Arc<crate::work::mail::pole::PoleMeta>,
    pub(crate) span: super::space::Span,
}

pub(crate) struct Construction(Arc<Team>);
impl Drop for Construction {
    fn drop(&mut self) {
        self.0.operating.store(false, Ordering::Release);
    }
}

impl Team {
    pub(crate) fn paused(&self) -> bool {
        if self.paused.load(Ordering::Acquire) { return true; }
        let mut parent = self.sire.upgrade();
        while let Some(task) = parent {
            let team = &task.ident.team;
            if team.paused.load(Ordering::Acquire) { return true; }
            parent = team.sire.upgrade();
        }
        false
    }
    pub(crate) fn set_paused(&self, paused: bool) { self.paused.store(paused, Ordering::Release); }
    pub(crate) fn revision(&self) -> usize { self.revision.load(Ordering::Acquire) }
    pub(crate) fn changed(&self) {
        self.revision.fetch_add(1, Ordering::Release);
        let mut parent = self.sire.upgrade();
        while let Some(task) = parent {
            task.ident.team.revision.fetch_add(1, Ordering::Release);
            parent = task.ident.team.sire.upgrade();
        }
    }
    pub(crate) fn representative(&self, task: TaskId) {
        self.completion.lock().get_or_insert(Completion { task, reason: None, fault: None });
    }
    pub(crate) fn completed(&self, task: TaskId, reason: env::Reason) {
        if let Some(result) = self.completion.lock().as_mut() {
            if reason != 0 && reason != crate::work::room::messenger::EXIT_DOOM && reason != crate::work::room::messenger::EXIT_CASCADE { result.fault.get_or_insert(reason); }
            if result.task == task { result.reason.get_or_insert(reason); }
        }
        if reason != 0 && reason != crate::work::room::messenger::EXIT_DOOM && reason != crate::work::room::messenger::EXIT_CASCADE
            && !self.observed.load(Ordering::Acquire) && self.observed() {
            if let Some(observer) = self.observer.as_ref().and_then(Weak::upgrade) {
                if let Some(result) = observer.completion.lock().as_mut() { result.fault.get_or_insert(reason); }
            }
            let mut parent = self.sire.upgrade();
            while let Some(task) = parent {
                let team = &task.ident.team;
                if let Some(result) = team.completion.lock().as_mut() { result.fault.get_or_insert(reason); }
                if team.observed.load(Ordering::Acquire) { break; }
                parent = team.sire.upgrade();
            }
        }
    }
    pub(crate) fn status(&self) -> (usize, env::Reason) {
        match self.completion.lock().as_ref() {
            Some(result) if result.reason.is_some() => (1, result.fault.or(result.reason).unwrap_or(0)),
            Some(result) if result.fault.is_some() => (2, result.fault.unwrap_or(0)),
            _ => (0, 0),
        }
    }
    pub(crate) fn observe(&self) { self.observed.store(true, Ordering::Release); }
    pub(crate) fn observed(&self) -> bool {
        if self.observed.load(Ordering::Acquire) || self.observer.is_some() { return true; }
        let mut parent = self.sire.upgrade();
        while let Some(task) = parent {
            if task.ident.team.observed.load(Ordering::Acquire) { return true; }
            parent = task.ident.team.sire.upgrade();
        }
        false
    }
    pub(crate) fn tasks_checked(&self) -> Result<Vec<TaskWeak>, env::UnitFail> {
        let tasks = self.tasks.lock(); let mut out = Vec::new();
        out.try_reserve(tasks.len()).map_err(|_| env::UnitFail::OoM)?;
        out.extend(tasks.iter().map(|task| task.copy_at(Site::Snapshot))); Ok(out)
    }
    pub(crate) fn operation(self: &Arc<Self>) -> Option<Construction> {
        self.operating
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        Some(Construction(self.clone()))
    }

    pub(crate) fn cancel_staging(&self) -> Result<(), crate::memory::manager::MapError> {
        loop {
            let item = self
                .staged
                .lock()
                .last()
                .map(|item| (item.meta.clone(), item.span));
            let Some((meta, span)) = item else { break };
            self.space.release(span)?;
            meta.backing().unreserve();
            self.staged.lock().pop();
        }
        Ok(())
    }
    pub(crate) fn prune_tasks(&self, exited: &Arc<Task>) {
        let exited_ptr = Arc::as_ptr(exited);
        self.tasks.lock().retain(|t| {
            if Weak::strong_count(t) == 0 {
                return false;
            }
            !(Weak::as_ptr(t) == exited_ptr)
        });
    }

    pub(crate) fn all_reaped(&self) -> bool {
        {
            let held = self.held.lock();
            if !held.is_empty() {
                return false;
            }
        }
        let g = self.tasks.lock();
        g.iter().all(|t| match t.upgrade() {
            Some(task) => {
                let reaped = task.tag() == TaskTag::Reaped;
                drop(task);
                reaped
            }
            None => true,
        })
    }

    pub(crate) fn tasks_snapshot(&self) -> Vec<TaskWeak> {
        let g = self.tasks.lock();
        let mut out: Vec<TaskWeak> = Vec::new();
        if out.try_reserve(g.len()).is_err() {
            return Vec::new();
        }
        out.extend(g.iter().map(|w| w.copy_at(Site::Snapshot)));
        out
    }

    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }

    pub(crate) fn release_held(&self, task: &Arc<Task>) -> bool {
        let mut g = self.held.lock();
        match g.iter().position(|t| Arc::ptr_eq(t, task)) {
            Some(i) => {
                g.swap_remove(i);
                true
            }
            None => false,
        }
    }

    pub(crate) fn default_entry(&self) -> usize {
        self.default_entry.get().copied().unwrap_or(0)
    }

    pub(crate) fn set_default_entry(&self, va: usize) {
        let _ = self.default_entry.set(va);
    }

    pub(crate) fn sire(&self) -> Option<TaskId> {
        self.sire.upgrade().map(|t| t.ident.id)
    }
}

impl Drop for Team {
    fn drop(&mut self) {
        self.cancel_staging().expect("team: cancel staging");
    }
}

pub struct TeamBuilder {
    space: Space,
    sire: TaskWeak,
    constructing: bool,
}

impl TeamBuilder {
    pub fn new(space: Space) -> TeamBuilder {
        TeamBuilder {
            space,
            sire: TaskWeak::empty(),
            constructing: false,
        }
    }

    pub fn sire(mut self, sire: TaskWeak) -> TeamBuilder {
        self.sire = sire;
        self
    }

    pub(crate) fn constructing(mut self) -> Self {
        self.constructing = true;
        self
    }

    pub fn spawn(self) -> Result<Arc<Team>, crate::memory::manager::MapError> {
        let id = alloc_team_id();
        let observer = self.sire.upgrade().and_then(|task| {
            if task.ident.team.observed.load(Ordering::Acquire) { Some(Arc::downgrade(&task.ident.team)) }
            else { task.ident.team.observer.clone() }
        });
        let space = crate::tag!(Space, Arc::try_new(self.space))
            .map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
        let team = crate::tag!(
            Team,
            Arc::try_new(Team {
                space,
                tasks: SpinLock::new_level(Level::TeamTasks, Vec::new()),
                held: SpinLock::new_level(Level::L3, Vec::new()),
                id,
                sire: self.sire,
                default_entry: OnceLock::new(),
                ready: AtomicBool::new(!self.constructing),
                operating: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                revision: AtomicUsize::new(0),
                completion: SpinLock::new(None),
                observed: AtomicBool::new(false),
                observer,
                staged: SpinLock::new(Vec::new()),
            })
        )
        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)?;
        if let Some(sire) = team.sire.upgrade() {
            sire.adopt(team.clone())
                .map_err(|()| crate::memory::manager::MapError::OutOfMemory)?;
        }
        team.changed();
        Ok(team)
    }
}

pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

pub(crate) fn init_kernel(space: Arc<Space>) -> Result<&'static Arc<Team>, crate::memory::manager::MapError> {
    if let Some(team) = KERNEL_TEAM.get() { return Ok(team) }
    let team = {
        let id = alloc_team_id();
        crate::tag!(
            Team,
            Arc::try_new(Team {
                space,
                tasks: SpinLock::new_level(Level::TeamTasks, Vec::new()),
                held: SpinLock::new_level(Level::L3, Vec::new()),
                id,
                sire: TaskWeak::empty(),
                default_entry: OnceLock::new(),
                ready: AtomicBool::new(true),
                operating: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                revision: AtomicUsize::new(0),
                completion: SpinLock::new(None),
                observed: AtomicBool::new(false),
                observer: None,
                staged: SpinLock::new(Vec::new()),
            })
        ).map_err(|_| crate::memory::manager::MapError::OutOfMemory)?
    };
    assert!(KERNEL_TEAM.set(team).is_ok(), "kernel team already initialized");
    Ok(KERNEL_TEAM.get().expect("kernel team just initialized"))
}

pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}

static NEXT_TEAM_ID: AtomicUsize = AtomicUsize::new(1);

pub(crate) fn alloc_team_id() -> TeamId {
    TeamId::new(NEXT_TEAM_ID.fetch_add(1, Ordering::Relaxed))
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
    let first = !target.ready.load(Ordering::Acquire);
    let _construction = if first {
        Some(target.operation().ok_or(UnitFail::Busy)?)
    } else {
        None
    };
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
    let caller_id = caller.map(|caller| caller.ident.id);
    let result = (|| -> Result<Arc<Task>, UnitFail> {
        use crate::work::unit::gate::{self, Permission};
        let mut prepared = builder.prepare().map_err(map_err)?;
        if !first {
            return prepared.publish(|| {}).map_err(map_err);
        }
        let caller = caller.ok_or(UnitFail::Denied)?;
        let mut staged = target.staged.lock();
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
        if *closed {
            return Err(UnitFail::Denied);
        }
        let mut pies = caller.pies.lock();
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
                    caller.heirs.lock().retain(|(parent, _, _)| *parent != item.token);
                    retired.push(pie);
                    item.meta.backing().unreserve();
                    leases.push(item);
                }
                gate::changed(caller);
                target.set_default_entry(entry_va);
                target
                    .ready
                    .store(true, core::sync::atomic::Ordering::Release);
            })
            .map_err(map_err);
        drop(pies);
        drop(closed);
        drop(staged);
        drop(operations);
        drop(retired);
        drop(leases);
        result
    })();
    // 首次提交会把调用者表里那几枚 staging 根移交给子域：出闭包、出任务的 `gate`、也出了
    // `NoAllocation` 之后才要求复核一次。
    if let Some(caller) = caller_id
        && first
        && result.is_ok()
    {
        let _ = crate::work::room::messenger::signal(
            crate::work::room::messenger::WakeKey::Capabilities { task: caller },
        );
    }
    result
}
