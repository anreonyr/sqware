use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::copies::Site;

use env::{PieToken, TaskId, UnitFail};

use crate::layout::{HART_FRAME_BASE, IMAGE_BASE, TASK_STACK_SIZE};
use crate::lock::{Level, SpinLock, SpinLockGuard};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::PhysAddr;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::{Frame, MapError};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::TrapContext;
use crate::work::pend::{CreationRegistry, Pie};
use crate::work::room::conductor;
use crate::work::unit::life::Life;

use crate::work::unit::space::{Mapping, RegionId, Reservation, Space};
use crate::work::unit::team::kernel;

use super::team::Team;
use crate::work::room::messenger::{Ticket, WakeKey};
use crate::work::room::scheduler;
use env::TeamId;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

#[cfg(debug_assertions)]
static FAIL_PREPARATION: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_preparation_at(stage: usize) {
    FAIL_PREPARATION.store(stage, Ordering::Relaxed);
}

#[cfg(debug_assertions)]
static FAIL_INVALIDATION: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_invalidation_at(stage: usize) {
    assert!(crate::testing());
    FAIL_INVALIDATION.store(stage, Ordering::Relaxed);
}

fn invalidation_checkpoint(_stage: usize, _tree: &crate::memory::manager::Tree) {
    #[cfg(debug_assertions)]
    if FAIL_INVALIDATION
        .compare_exchange(_stage, 0, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        crate::memory::manager::asid::fail_next_shootdown(_tree.asid());
    }
}

#[cfg(debug_assertions)]
static FAIL_ROLLBACK_FENCE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_rollback_fence() {
    assert!(crate::testing());
    FAIL_ROLLBACK_FENCE.store(true, Ordering::Relaxed);
}

#[inline]
fn preparation_checkpoint(_stage: usize) -> Result<(), MapError> {
    #[cfg(debug_assertions)]
    if FAIL_PREPARATION
        .compare_exchange(_stage, 0, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        return Err(MapError::OutOfMemory);
    }
    Ok(())
}

pub(crate) const MAX_ARGS: usize = 64;

pub enum TaskState {
    Running {
        hart: crate::hart::HartId,
        ticks_left: u32,
    },
    Blocked {
        key: WakeKey,
        ticket: Ticket,
        next: Option<Arc<Task>>,
        wait: Option<crate::work::room::messenger::WaitCall>,
    },
    Starved {
        next: Option<Arc<Task>>,
    },
    Held,
    Parked,
    Debarking {
        hart: crate::hart::HartId,
        ticks_left: u32,
    },
    Debarked {
        state: TaskStopped,
    },
    Doomed {
        hart: Option<crate::hart::HartId>,
        cause: TaskExitCause,
        reason: usize,
    },
    Reaped {
        cause: TaskExitCause,
        reason: usize,
    },
}
pub enum TaskStopped {
    Held,
    Starved,
    Blocked {
        key: WakeKey,
        ticket: Ticket,
        next: Option<Arc<Task>>,
        wait: Option<crate::work::room::messenger::WaitCall>,
    },
}

pub use env::ExitCause as TaskExitCause;

pub use env::TaskExit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTag {
    Held,
    Starved,
    Running,
    Parked,
    Debarking,
    Debarked,
    Blocked,
    Doomed,
    Reaped,
}

impl core::fmt::Debug for TaskState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TaskState::Starved { next } => write!(f, "Starved {{ next: {} }}", next.is_some()),
            TaskState::Reaped { cause, reason } => {
                write!(f, "Reaped {{ cause: {cause:?}, reason: {reason:#x} }}")
            }
            TaskState::Blocked {
                key, ticket, next, ..
            } => write!(
                f,
                "Blocked {{ key: {:?}, ticket: {:?}, next: {} }}",
                key,
                ticket,
                next.is_some()
            ),
            other => write!(f, "{:?}", other.tag()),
        }
    }
}

impl TaskState {
    pub fn tag(&self) -> TaskTag {
        match self {
            TaskState::Held => TaskTag::Held,
            TaskState::Starved { .. } => TaskTag::Starved,
            TaskState::Running { .. } => TaskTag::Running,
            TaskState::Blocked { .. } => TaskTag::Blocked,
            TaskState::Parked => TaskTag::Parked,
            TaskState::Debarking { .. } => TaskTag::Debarking,
            TaskState::Debarked { .. } => TaskTag::Debarked,
            TaskState::Doomed { .. } => TaskTag::Doomed,
            TaskState::Reaped { .. } => TaskTag::Reaped,
        }
    }
}

pub struct Task {
    pub(crate) ident: Arc<TaskIdent>,
    pub(crate) life: Arc<Life>,
    pub(crate) state: Arc<SpinLock<TaskState>>,
    reap_next: SpinLock<Option<Arc<Task>>>,
    pub(crate) pend: Pend,
    pub(crate) heir: SpinLock<Vec<Arc<Team>>>,
}

pub(crate) struct Pend {
    serial: SpinLock<()>,
    pub(crate) version: AtomicUsize,
    pub(crate) pies: SpinLock<Vec<Pie>>,
    pub(crate) heirs: SpinLock<Vec<(PieToken, Weak<Task>, PieToken)>>,
    pub(crate) creations: CreationRegistry,
}
impl Pend {
    fn new() -> Self {
        Self {
            serial: SpinLock::new(()),
            version: AtomicUsize::new(0),
            pies: SpinLock::new(Vec::new()),
            heirs: SpinLock::new(Vec::new()),
            creations: CreationRegistry::new(),
        }
    }
    pub(crate) fn lock(&self) -> crate::lock::spin::SpinLockGuard<'_, ()> {
        self.serial.lock()
    }
}

unsafe impl Send for Task {}

#[repr(C)]
pub(crate) struct TaskWeak {
    w: Weak<Task>,
    id: usize,
    site: Site,
}

impl TaskWeak {
    pub(crate) fn stored(w: Weak<Task>, site: Site) -> TaskWeak {
        #[cfg(debug_assertions)]
        let id = super::copies::register(site);
        #[cfg(not(debug_assertions))]
        let id = 0;
        TaskWeak { w, id, site }
    }

    pub(crate) fn copy_at(&self, site: Site) -> TaskWeak {
        TaskWeak::stored(self.w.clone(), site)
    }

    pub(crate) fn empty() -> TaskWeak {
        TaskWeak {
            w: Weak::new(),
            id: 0,
            site: Site::Empty,
        }
    }
}

impl Deref for TaskWeak {
    type Target = Weak<Task>;
    fn deref(&self) -> &Weak<Task> {
        &self.w
    }
}

impl Drop for TaskWeak {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        super::copies::release(self.id);
    }
}

pub(crate) struct TaskIdent {
    pub(crate) id: TaskId,
    pub(crate) team: Arc<Team>,
    pub(crate) frame_pa: PhysAddr,
    pub(crate) frame_va: VirtAddr,
    memory: SpinLock<TaskMemory>,
}

struct TaskMemory {
    space: Arc<Space>,
    stack: Option<Mapping>,
    frame: Option<Mapping>,
    guard: Option<Reservation>,
}

impl TaskMemory {
    fn retire(&mut self) -> Result<(), MapError> {
        if let Some(mapping) = &self.frame {
            self.space.unmap(mapping)?;
            self.frame = None;
        }
        if let Some(mapping) = &self.stack {
            self.space.unmap(mapping)?;
            self.stack = None;
        }
        if let Some(guard) = self.guard.take() {
            self.space.release_reservation(guard)?;
        }
        Ok(())
    }
}

impl Drop for TaskMemory {
    fn drop(&mut self) {
        if let Some(mapping) = self.frame.take() {
            let _ = self.space.dispose(mapping);
        }
        if let Some(mapping) = self.stack.take() {
            let _ = self.space.dispose(mapping);
        }
        if let Some(guard) = self.guard.take() {
            let _ = self.space.release_reservation(guard);
        }
    }
}

impl TaskIdent {
    pub(crate) fn retire(&self) -> Result<(), MapError> {
        self.memory.lock().retire()
    }
}

pub(crate) struct PreparedTask {
    ident: Arc<TaskIdent>,
    life: Arc<Life>,
    state: Arc<SpinLock<TaskState>>,
    slot: Option<Arc<MaybeUninit<Task>>>,
}

impl PreparedTask {
    pub(crate) fn publish(&mut self, commit: impl FnOnce()) -> Result<Arc<Task>, MapError> {
        let team = self.ident.team.clone();
        let receipts = team.receipts()?;
        let member = Arc::try_new(super::team::Member {
            id: self.ident.id,
            task: TaskWeak::empty(),
            node: Arc::downgrade(&team.life),
            state: self.state.clone(),
            life: self.life.clone(),
            receipts: SpinLock::new(receipts),
        })
        .map_err(|_| MapError::OutOfMemory)?;
        scheduler::core::reserve_publication().map_err(|_| MapError::OutOfMemory)?;
        {
            let mut tasks = team.tasks.lock();
            if tasks.len() >= super::team::MAX_MEMBERS {
                return Err(MapError::OutOfMemory);
            }
            let mut held = team.tasks.held.lock();
            tasks.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
            let held_extra = tasks.len() + 1 - held.len();
            held.try_reserve(held_extra)
                .map_err(|_| MapError::OutOfMemory)?;
        }
        let _commit = super::commit();
        if team.life.closed() {
            return Err(MapError::NoRegion);
        }
        let mut tasks = team.tasks.lock();
        let mut held = team.tasks.held.lock();
        if tasks.len() >= super::team::MAX_MEMBERS
            || tasks.len() == tasks.capacity()
            || held.capacity() < tasks.len() + 1
        {
            return Err(MapError::OutOfMemory);
        }
        let (task, ()) = scheduler::core::publish(|| {
            commit();
            assert!(team.ready(), "publish: constructing team");
            let mut slot = self.slot.take().expect("prepare slot");
            Arc::get_mut(&mut slot)
                .expect("unique prepare slot")
                .write(Task {
                    ident: self.ident.clone(),
                    life: self.life.clone(),
                    state: self.state.clone(),
                    reap_next: SpinLock::new(None),
                    pend: Pend::new(),
                    heir: SpinLock::new(Vec::new()),
                });
            let task = unsafe { slot.assume_init() };
            let mut member = member;
            Arc::get_mut(&mut member).expect("private member").task =
                TaskWeak::stored(Arc::downgrade(&task), Site::TeamTasks);
            tasks.push(member);
            held.push(task.clone());
            conductor::push();
            (task, ())
        })
        .map_err(|_| MapError::OutOfMemory)?;

        drop(held);
        drop(tasks);

        trace::note(EventKind::Room(RoomEvent::Spawn {
            tid: task.ident.id.get(),
        }));
        Ok(task)
    }
}

impl Task {
    pub(crate) fn transform(&self, next: TaskState) {
        let mut state = self.state.lock();
        let legal = match (&*state, &next) {
            (TaskState::Held, TaskState::Starved { .. })
            | (TaskState::Starved { .. }, TaskState::Running { .. })
            | (TaskState::Running { .. }, TaskState::Starved { .. })
            | (TaskState::Doomed { .. }, TaskState::Reaped { .. }) => true,
            (old, TaskState::Doomed { .. }) => {
                !matches!(old, TaskState::Doomed { .. } | TaskState::Reaped { .. })
            }
            _ => false,
        };
        assert!(legal, "illegal task state transition");
        if matches!(&next, TaskState::Reaped { .. }) {
            debug_assert!(self.reap_next.lock().is_none(), "退出前仍挂在回收链上");
        }
        let unlinked = match &next {
            TaskState::Starved { next } => next.is_none(),
            TaskState::Blocked { next, .. } => next.is_none(),
            _ => true,
        };
        debug_assert!(unlinked, "transform: 入链载荷非空（有人没先摘链）");
        *state = next;
    }

    pub(crate) fn state(&self) -> SpinLockGuard<'_, TaskState> {
        self.state.lock()
    }

    pub(crate) fn starved_next(t: &Arc<Self>) -> Option<Arc<Task>> {
        match &*t.state.lock() {
            TaskState::Starved { next } => next.clone(),
            other => unreachable!("就绪链只穿 Starved 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn set_starved_next(t: &Arc<Self>, value: Option<Arc<Task>>) {
        match &mut *t.state.lock() {
            TaskState::Starved { next } => *next = value,
            other => unreachable!("就绪链只穿 Starved 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn take_starved_next(t: &Arc<Self>) -> Option<Arc<Task>> {
        match &mut *t.state.lock() {
            TaskState::Starved { next } => next.take(),
            other => unreachable!("就绪链只穿 Starved 任务，实为 {:?}", other.tag()),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn reaped_next(t: &Arc<Self>) -> Option<Arc<Task>> {
        debug_assert!(matches!(&*t.state.lock(), TaskState::Reaped { .. }));
        t.reap_next.lock().as_ref().cloned()
    }

    pub(crate) fn set_reaped_next(t: &mut Arc<Self>, value: Option<Arc<Task>>) {
        debug_assert!(matches!(&*t.state.lock(), TaskState::Reaped { .. }));
        *t.reap_next.lock() = value;
    }

    pub(crate) fn take_reaped_next(t: &mut Arc<Self>) -> Option<Arc<Task>> {
        debug_assert!(matches!(&*t.state.lock(), TaskState::Reaped { .. }));
        t.reap_next.lock().take()
    }

    pub(crate) fn blocked_next(t: &Arc<Self>) -> Option<Arc<Task>> {
        match &*t.state.lock() {
            TaskState::Blocked { next, .. }
            | TaskState::Debarked {
                state: TaskStopped::Blocked { next, .. },
            } => next.clone(),
            other => unreachable!("等待链只穿 Blocked 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn set_blocked_next(t: &mut Arc<Self>, value: Option<Arc<Task>>) {
        match &mut *t.state.lock() {
            TaskState::Blocked { next, .. }
            | TaskState::Debarked {
                state: TaskStopped::Blocked { next, .. },
            } => *next = value,
            other => unreachable!("等待链只穿 Blocked 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn take_blocked_next(t: &mut Arc<Self>) -> Option<Arc<Task>> {
        match &mut *t.state.lock() {
            TaskState::Blocked { next, .. }
            | TaskState::Debarked {
                state: TaskStopped::Blocked { next, .. },
            } => next.take(),
            other => unreachable!("等待链只穿 Blocked 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn blocked_ticket(t: &Arc<Self>) -> Ticket {
        match &*t.state.lock() {
            TaskState::Blocked { ticket, .. }
            | TaskState::Debarked {
                state: TaskStopped::Blocked { ticket, .. },
            } => *ticket,
            other => unreachable!("等待链只穿 Blocked 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn take_wait(&self) -> Option<crate::work::room::messenger::WaitCall> {
        match &mut *self.state.lock() {
            TaskState::Blocked { wait, .. }
            | TaskState::Debarked {
                state: TaskStopped::Blocked { wait, .. },
            } => wait.take(),
            _ => None,
        }
    }

    pub fn tag(&self) -> TaskTag {
        self.state.lock().tag()
    }

    pub(crate) fn dec_ticks_left(&self) {
        let mut state = self.state.lock();
        match &*state {
            TaskState::Running { ticks_left, .. } | TaskState::Debarking { ticks_left, .. } => {
                debug_assert!(*ticks_left >= 1, "Running 预算恒 ≥ 1");
                let remaining = *ticks_left - 1;
                match &mut *state {
                    TaskState::Running { ticks_left, .. }
                    | TaskState::Debarking { ticks_left, .. } => *ticks_left = remaining,
                    _ => unreachable!(),
                };
            }
            _ => unreachable!("dec_ticks_left 只对 Running 任务调用"),
        }
    }

    pub(crate) fn release(task: &Arc<Task>) -> Result<(), UnitFail> {
        let _commit = super::commit();
        if task.tag() != TaskTag::Held || !task.ident.team.release_held(task) {
            return Err(UnitFail::Denied);
        }
        *task.state.lock() = TaskState::Starved { next: None };
        scheduler::core::launch(task.clone());
        Ok(())
    }
    pub(crate) fn debark(task: &Arc<Task>) -> Result<(), UnitFail> {
        let _commit = super::commit();
        match task.tag() {
            TaskTag::Held => {
                *task.state.lock() = TaskState::Debarked {
                    state: TaskStopped::Held,
                }
            }
            TaskTag::Starved => {
                if !scheduler::core::remove_from_starved(task) {
                    return Err(UnitFail::Busy);
                }
                *task.state.lock() = TaskState::Debarked {
                    state: TaskStopped::Starved,
                };
                task.ident.team.park(task.clone());
            }
            TaskTag::Parked => {
                *task.state.lock() = TaskState::Debarked {
                    state: TaskStopped::Starved,
                }
            }
            TaskTag::Blocked => {
                let mut state = task.state.lock();
                let old = core::mem::replace(&mut *state, TaskState::Parked);
                let TaskState::Blocked {
                    key,
                    ticket,
                    next,
                    wait,
                } = old
                else {
                    unreachable!()
                };
                *state = TaskState::Debarked {
                    state: TaskStopped::Blocked {
                        key,
                        ticket,
                        next,
                        wait,
                    },
                };
            }
            TaskTag::Running => {
                let mut state = task.state.lock();
                let TaskState::Running { hart, ticks_left } = &*state else {
                    unreachable!()
                };
                let hart = *hart;
                let ticks_left = *ticks_left;
                *state = TaskState::Debarking { hart, ticks_left };
                drop(state);
                crate::work::room::conductor::nudge(hart);
                return Err(UnitFail::Busy);
            }
            TaskTag::Debarking => return Err(UnitFail::Busy),
            TaskTag::Debarked => {}
            _ => return Err(UnitFail::Denied),
        }
        Ok(())
    }
    pub(crate) fn embark(task: &Arc<Task>) -> Result<(), UnitFail> {
        let _commit = super::commit();
        if task.tag() == TaskTag::Held {
            return Self::release(task);
        }
        let mut state = task.state.lock();
        let held = match &*state {
            TaskState::Debarked {
                state: TaskStopped::Held,
            } => true,
            TaskState::Debarked {
                state: TaskStopped::Starved,
            } => false,
            TaskState::Debarked {
                state: TaskStopped::Blocked { .. },
            } => {
                let old = core::mem::replace(&mut *state, TaskState::Parked);
                let TaskState::Debarked {
                    state:
                        TaskStopped::Blocked {
                            key,
                            ticket,
                            next,
                            wait,
                        },
                } = old
                else {
                    unreachable!()
                };
                *state = TaskState::Blocked {
                    key,
                    ticket,
                    next,
                    wait,
                };
                return Ok(());
            }
            TaskState::Debarking { .. } => return Err(UnitFail::Busy),
            TaskState::Doomed { .. } | TaskState::Reaped { .. } => return Err(UnitFail::Denied),
            _ => return Err(UnitFail::Busy),
        };
        *state = if held {
            TaskState::Held
        } else {
            TaskState::Starved { next: None }
        };
        drop(state);
        if held {
            Self::release(task)
        } else {
            task.ident.team.release_held(task);
            scheduler::core::launch(task.clone());
            Ok(())
        }
    }
    pub(crate) fn stopped(&self) -> bool {
        matches!(
            &*self.state.lock(),
            TaskState::Debarking { .. } | TaskState::Debarked { .. }
        )
    }
    pub(crate) fn park(task: Arc<Task>) {
        let team = task.ident.team.clone();
        let mut state = task.state.lock();
        *state = if matches!(
            &*state,
            TaskState::Debarking { .. } | TaskState::Debarked { .. }
        ) {
            TaskState::Debarked {
                state: TaskStopped::Starved,
            }
        } else {
            TaskState::Parked
        };
        drop(state);
        team.park(task);
    }
    pub(crate) fn rise(task: &Arc<Task>) -> bool {
        let mut state = task.state.lock();
        let stopped = matches!(&*state, TaskState::Debarked { .. });
        *state = if stopped {
            TaskState::Debarked {
                state: TaskStopped::Starved,
            }
        } else {
            TaskState::Starved { next: None }
        };
        drop(state);
        if stopped {
            task.ident.team.park(task.clone());
            false
        } else {
            true
        }
    }

    fn count_vanished(&self) {
        if self.tag() != TaskTag::Reaped {
            crate::work::room::conductor::exit();
        }
    }

    pub(crate) fn adopt(&self, child: Arc<Team>) -> Result<(), ()> {
        {
            self.heir.lock().try_reserve(1).map_err(|_| ())?;
        }
        let _commit = super::commit();
        if matches!(self.tag(), TaskTag::Doomed | TaskTag::Reaped) {
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
        let _commit = super::commit();
        let child = {
            let mut heirs = self.heir.lock();
            let at = heirs.iter().position(|t| t.id == team)?;
            heirs.remove(at)
        };
        *child.state.lock() = super::team::TeamState::Ousted;
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
        let _commit = super::commit();
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
        found.filter(|team| !matches!(&*team.state.lock(), super::team::TeamState::Ousted))
    }

    #[cfg(debug_assertions)]
    pub(crate) fn heir_count(&self) -> usize {
        self.heir.lock().len()
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }
}

fn write_args(space: &crate::work::unit::space::Space, at: VirtAddr, args: &[usize]) {
    let mut done = 0usize;
    while done < args.len() {
        let va = at + done * size_of::<usize>();
        let (pa, _) = space
            .tree
            .translate(va)
            .expect("stack page materialized before args write");
        let page_rest = PAGE_SIZE - (va.as_usize() % PAGE_SIZE);
        let n = core::cmp::min((args.len() - done) * size_of::<usize>(), page_rest)
            / size_of::<usize>();
        let dst = pa.as_usize() as *mut usize;
        for i in 0..n {
            unsafe { core::ptr::write_volatile(dst.add(i), args[done + i]) };
        }
        done += n;
    }
}

pub struct TaskBuilder {
    team: Arc<Team>,
    entry: VirtAddr,
    args: Vec<usize>,
    stack: usize,
}

impl TaskBuilder {
    pub fn new(team: Arc<Team>) -> TaskBuilder {
        let entry = match team.default_entry() {
            0 => IMAGE_BASE,
            e => VirtAddr::from_raw(e),
        };
        TaskBuilder {
            team,
            entry,
            args: Vec::new(),
            stack: TASK_STACK_SIZE,
        }
    }

    pub fn args(mut self, args: Vec<usize>) -> TaskBuilder {
        debug_assert!(args.len() <= MAX_ARGS, "args 超过 MAX_ARGS");
        self.args = args;
        self
    }

    pub fn entry(mut self, entry: VirtAddr) -> TaskBuilder {
        self.entry = entry;
        self
    }

    pub fn stack(mut self, size: usize) -> TaskBuilder {
        self.stack = size;
        self
    }

    pub fn hold(self) -> Result<Arc<Task>, MapError> {
        if !self.team.ready() {
            return Err(MapError::WidenDenied);
        }
        self.prepare()?.publish(|| {})
    }

    pub(crate) fn prepare(self) -> Result<PreparedTask, MapError> {
        let stack_size = self
            .stack
            .max(1)
            .checked_next_multiple_of(PAGE_SIZE)
            .filter(|size| size.checked_add(crate::layout::TASK_STACK_GUARD).is_some())
            .ok_or(MapError::NoRegion)?;
        let id = TaskId::new(
            NEXT_ID
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                    (id < isize::MAX as usize).then_some(id + 1)
                })
                .map_err(|_| MapError::OutOfMemory)?,
        );

        let mut memory = TaskMemory {
            space: self.team.space.clone(),
            stack: None,
            frame: None,
            guard: None,
        };
        let slot = self.team.space.allocate_high(
            RegionId::Normal,
            stack_size + crate::layout::TASK_STACK_GUARD,
        )?;
        let (guard, body) = self
            .team
            .space
            .split_reservation(slot, crate::layout::TASK_STACK_GUARD)?;
        memory.guard = Some(guard);
        let stack_body_top = body.start().as_usize() + body.size();
        let mut frames = Vec::new();
        if frames.try_reserve_exact(stack_size / PAGE_SIZE).is_err() {
            self.team.space.release_reservation(body)?;
            return Err(MapError::OutOfMemory);
        }
        for _ in 0..stack_size / PAGE_SIZE {
            match Frame::allocate() {
                Ok(frame) => frames.push(frame),
                Err(error) => {
                    self.team.space.release_reservation(body)?;
                    return Err(error);
                }
            }
        }
        let rw =
            crate::memory::manager::entry::PteFlags::R | crate::memory::manager::entry::PteFlags::W;
        let active = rw
            | crate::memory::manager::entry::PteFlags::A
            | crate::memory::manager::entry::PteFlags::D;
        invalidation_checkpoint(1, &self.team.space.tree);
        memory.stack = Some(self.team.space.map_frames(body, frames, rw, active)?);
        preparation_checkpoint(1)?;

        let mut frame_pages = Vec::new();
        frame_pages
            .try_reserve_exact(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let frame = Frame::allocate()?;
        let frame_pa = PhysAddr::from_raw(frame.physical_address());
        frame_pages.push(frame);
        let reservation = self.team.space.allocate_low(RegionId::Kernel, PAGE_SIZE)?;
        let frame_va = reservation.start();
        invalidation_checkpoint(2, &self.team.space.tree);
        memory.frame = Some(self.team.space.map_frames(
            reservation,
            frame_pages,
            rw,
            active - crate::memory::manager::entry::PteFlags::U,
        )?);
        preparation_checkpoint(2)?;

        let count = self.args.len();
        let args_at = stack_body_top - count * size_of::<usize>();
        if count > 0 {
            write_args(&self.team.space, VirtAddr::from_raw(args_at), &self.args);
        }
        let sp = VirtAddr::wrap(args_at & !0xF);

        let frame = unsafe { &mut *(frame_pa.as_usize() as *mut TrapContext) };
        unsafe {
            let ktc = kernel()
                .expect("kernel team not initialized")
                .space
                .tree
                .translate(HART_FRAME_BASE)
                .expect("kernel frame not mapped")
                .0
                .as_usize() as *const TrapContext;
            frame.init(
                &*ktc,
                &self.team,
                self.entry,
                sp,
                (VirtAddr::wrap(args_at), count),
                frame_pa,
                frame_va,
            );
        }

        let alloc = crate::memory::allocator::hybrid::allocator();
        let ident: Arc<TaskIdent> = unsafe {
            let ident = crate::tag!(
                Task,
                Arc::try_new_in(
                    TaskIdent {
                        id,
                        team: self.team.clone(),
                        frame_pa,
                        frame_va,
                        memory: SpinLock::new(memory),
                    },
                    alloc,
                )
            )
            .map_err(|_| MapError::OutOfMemory)?;
            let (ptr, _alloc) = Arc::into_raw_with_allocator(ident);
            Arc::from_raw(ptr)
        };
        #[cfg(debug_assertions)]
        if FAIL_ROLLBACK_FENCE.swap(false, Ordering::Relaxed) {
            crate::memory::manager::asid::fail_next_shootdown(self.team.space.tree.asid());
        }
        preparation_checkpoint(3)?;
        let life = Life::try_new().map_err(|_| MapError::OutOfMemory)?;
        preparation_checkpoint(4)?;
        let state = Arc::try_new(SpinLock::new_level(Level::UnitState, TaskState::Held))
            .map_err(|_| MapError::OutOfMemory)?;
        let slot: Arc<MaybeUninit<Task>> = unsafe {
            let slot = crate::tag!(Task, Arc::<Task, _>::try_new_uninit_in(alloc))
                .map_err(|_| MapError::OutOfMemory)?;
            let (ptr, _) = Arc::into_raw_with_allocator(slot);
            Arc::from_raw(ptr)
        };
        Ok(PreparedTask {
            ident,
            life,
            state,
            slot: Some(slot),
        })
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let _commit = super::commit();
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
