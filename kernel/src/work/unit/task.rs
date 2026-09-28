use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use env::{TaskId, UnitFail};

use crate::layout::{HART_FRAME_BASE, IMAGE_BASE, TASK_STACK_SIZE};
use crate::lock::SpinLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::TrapContext;
use crate::work::room::conductor;
use crate::work::unit::gate::AnyPie;
use crate::work::unit::life::Life;

use crate::work::unit::space::window::{FrameWindow, StackWindow};
use crate::work::unit::team::kernel;

use super::team::Team;
use crate::work::room::messenger::{Ticket, WakeKey};
use crate::work::room::scheduler;
use env::TeamId;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

pub(crate) const MAX_ARGS: usize = 64;

pub enum TaskState {
    Running { ticks_left: u32 },
    Blocked {
        key: WakeKey,
        ticket: Ticket,
        next: Option<Arc<Task>>,
    },
    Starved {
        next: Option<Arc<Task>>,
    },
    Held,
    Doomed,
    Reaped {
        next: Option<Arc<Task>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskTag {
    Held = 0,
    Starved = 1,
    Running = 2,
    Blocked = 3,
    Doomed = 4,
    Reaped = 5,
}

impl TaskTag {
    fn of(byte: u8) -> TaskTag {
        match byte {
            b if b == TaskTag::Held as u8 => TaskTag::Held,
            b if b == TaskTag::Starved as u8 => TaskTag::Starved,
            b if b == TaskTag::Running as u8 => TaskTag::Running,
            b if b == TaskTag::Blocked as u8 => TaskTag::Blocked,
            b if b == TaskTag::Doomed as u8 => TaskTag::Doomed,
            b if b == TaskTag::Reaped as u8 => TaskTag::Reaped,
            other => panic!("task tag 越界: {other}"),
        }
    }
}

impl core::fmt::Debug for TaskState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TaskState::Starved { next } => write!(f, "Starved {{ next: {} }}", next.is_some()),
            TaskState::Reaped { next } => write!(f, "Reaped {{ next: {} }}", next.is_some()),
            TaskState::Blocked { key, ticket, next } => write!(
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
            TaskState::Doomed => TaskTag::Doomed,
            TaskState::Reaped { .. } => TaskTag::Reaped,
        }
    }
}

pub struct Task {
    pub(crate) ident: Arc<TaskIdent>,
    pub(crate) life: Arc<Life>,
    state: TaskState,
    tag: AtomicU8,
    pub(crate) pies: SpinLock<Vec<AnyPie>>,
    pub(crate) heir: SpinLock<Vec<Arc<Team>>>,
}

pub(crate) struct TaskIdent {
    pub(crate) id: TaskId,
    pub(crate) team: Arc<Team>,
    pub(crate) stack: crate::work::unit::space::Span,
    pub(crate) frame: crate::work::unit::space::Span,
}

impl Task {
    pub(crate) fn transform(&mut self, next: TaskState) {
        let legal = matches!(
            (&self.state, &next),
            (TaskState::Held, TaskState::Starved { .. })
                | (TaskState::Starved { .. }, TaskState::Running { .. })
                | (TaskState::Running { .. }, TaskState::Starved { .. })
                | (TaskState::Running { .. }, TaskState::Blocked { .. })
                | (TaskState::Blocked { .. }, TaskState::Starved { .. })
                | (TaskState::Held, TaskState::Doomed)
                | (TaskState::Starved { .. }, TaskState::Doomed)
                | (TaskState::Blocked { .. }, TaskState::Doomed)
                | (TaskState::Running { .. }, TaskState::Doomed)
                | (TaskState::Doomed, TaskState::Reaped { .. })
        );
        assert!(
            legal,
            "illegal task state transform: {:?} -> {:?}",
            self.state, next
        );
        let unlinked = match &next {
            TaskState::Starved { next } | TaskState::Reaped { next } => next.is_none(),
            TaskState::Blocked { next, .. } => next.is_none(),
            _ => true,
        };
        debug_assert!(unlinked, "transform: 入链载荷非空（有人没先摘链）");
        let tag = next.tag() as u8;
        self.state = next;
        self.tag.store(tag, Ordering::Release);
    }

    pub(crate) fn state(&mut self) -> &TaskState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TaskState {
        &mut self.state
    }

    pub(crate) fn starved_next(t: &mut Arc<Self>) -> &mut Option<Arc<Task>> {
        match Self::exclusive(t).state_mut() {
            TaskState::Starved { next } => next,
            other => unreachable!("就绪链只穿 Starved 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn reaped_next(t: &mut Arc<Self>) -> &mut Option<Arc<Task>> {
        match Self::exclusive(t).state_mut() {
            TaskState::Reaped { next } => next,
            other => unreachable!("躯壳链只穿 Reaped 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn blocked_next(t: &mut Arc<Self>) -> &mut Option<Arc<Task>> {
        match Self::exclusive(t).state_mut() {
            TaskState::Blocked { next, .. } => next,
            other => unreachable!("等待链只穿 Blocked 任务，实为 {:?}", other.tag()),
        }
    }

    pub(crate) fn blocked_ticket(t: &mut Arc<Self>) -> Ticket {
        match Self::exclusive(t).state_mut() {
            TaskState::Blocked { ticket, .. } => *ticket,
            other => unreachable!("等待链只穿 Blocked 任务，实为 {:?}", other.tag()),
        }
    }

    pub fn tag(&self) -> TaskTag {
        TaskTag::of(self.tag.load(Ordering::Acquire))
    }

    pub(crate) fn dec_ticks_left(&mut self) {
        match self.state {
            TaskState::Running { ticks_left } => {
                debug_assert!(ticks_left >= 1, "Running 预算恒 ≥ 1");
                self.state = TaskState::Running {
                    ticks_left: ticks_left - 1,
                };
            }
            _ => unreachable!("dec_ticks_left 只对 Running 任务调用"),
        }
    }

    pub(crate) fn exclusive(t: &mut Arc<Self>) -> &mut Task {
        #[cfg(debug_assertions)]
        assert!(
            Arc::strong_count(t) >= 1,
            "task #{}: no holders (strong_count == 0)",
            t.ident.id.get()
        );
        // SAFETY: 至少一个容器持强引用
        unsafe { &mut *Arc::as_ptr(t).cast_mut() }
    }

    pub(crate) fn release(task: &Arc<Task>) -> Result<(), UnitFail> {
        let team = task.ident.team.clone();
        if !team.release_held(task) {
            return Err(UnitFail::Denied);
        }
        let mut t = task.clone();
        Task::exclusive(&mut t).transform(TaskState::Starved { next: None });
        scheduler::core::launch(t);
        Ok(())
    }

    fn count_vanished(&self) {
        if self.tag() != TaskTag::Reaped {
            crate::work::room::conductor::exit();
        }
    }

    pub(crate) fn adopt(&self, child: Arc<Team>) -> Result<(), ()> {
        let mut g = self.heir.lock();
        g.try_reserve(1).map_err(|_| ())?;
        g.push(child);
        Ok(())
    }

    pub(crate) fn oust(&self, team: TeamId) -> Option<Arc<Team>> {
        let mut g = self.heir.lock();
        let at = g.iter().position(|t| t.id == team)?;
        Some(g.remove(at))
    }

    pub(crate) fn heirs(&self) -> Vec<Arc<Team>> {
        self.heir.lock().clone()
    }

    pub(crate) fn heir(&self, id: TeamId) -> Option<Arc<Team>> {
        self.heir.lock().iter().find(|t| t.id == id).cloned()
    }

    pub(crate) fn heir_count(&self) -> usize {
        self.heir.lock().len()
    }

    pub(crate) fn heir_at(&self, index: usize) -> Option<TeamId> {
        self.heir.lock().get(index).map(|t| t.id)
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
            .translate(va)
            .expect("stack page materialized before args write");
        let page_rest = PAGE_SIZE - (va.as_usize() % PAGE_SIZE);
        let n = core::cmp::min((args.len() - done) * size_of::<usize>(), page_rest)
            / size_of::<usize>();
        let dst = pa.as_usize() as *mut usize;
        for i in 0..n {
            // SAFETY: 帧由本空间独占持有
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
        self.stack = size.max(1).next_multiple_of(PAGE_SIZE);
        self
    }

    pub fn hold(self) -> Result<Arc<Task>, MapError> {
        let id = TaskId::new(NEXT_ID.fetch_add(1, Ordering::Relaxed));

        scheduler::core::try_reserve_roster().map_err(|()| MapError::OutOfMemory)?;
        self.team
            .tasks
            .lock()
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        self.team
            .held
            .lock()
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;

        let stack_size = self.stack;
        let stack_span = StackWindow::claim(&self.team.space, stack_size)?;
        let stack_body = stack_span.va + crate::layout::TASK_STACK_GUARD;
        let stack_body_top = stack_body.as_usize() + stack_size;

        let frame_span = match FrameWindow::claim(&self.team.space) {
            Ok(s) => s,
            Err(e) => {
                self.team
                    .space
                    .release(stack_span)
                    .expect("release: rollback");
                return Err(e);
            }
        };
        let frame_pa = frame_span.pa.expect("frame span has pa");
        let frame_va = frame_span.va;

        let count = self.args.len();
        let args_at = stack_body_top - count * size_of::<usize>();
        if count > 0 {
            write_args(&self.team.space, VirtAddr::from_raw(args_at), &self.args);
        }
        let sp = VirtAddr::from_raw(args_at & !0xF);

        let frame = unsafe { &mut *(frame_pa.as_usize() as *mut TrapContext) };
        unsafe {
            let ktc = kernel()
                .expect("kernel team not initialized")
                .space
                .translate(HART_FRAME_BASE)
                .expect("kernel frame not mapped")
                .0
                .as_usize() as *const TrapContext;
            frame.init(
                &*ktc,
                &self.team,
                self.entry,
                sp,
                (VirtAddr::from_raw(args_at), count),
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
                        stack: stack_span,
                        frame: frame_span,
                    },
                    alloc,
                )
            )
            .map_err(|_| {
                self.team
                    .space
                    .release(frame_span)
                    .expect("release: rollback");
                self.team
                    .space
                    .release(stack_span)
                    .expect("release: rollback");
                MapError::OutOfMemory
            })?;
            let (ptr, _alloc) = Arc::into_raw_with_allocator(ident);
            Arc::from_raw(ptr)
        };
        let life = match Life::try_new() {
            Ok(l) => l,
            Err(_) => {
                drop(ident);
                return Err(MapError::OutOfMemory);
            }
        };
        let task: Arc<Task> = unsafe {
            let task = crate::tag!(
                Task,
                Arc::try_new_in(
                    Task {
                        ident,
                        life,
                        state: TaskState::Held,
                        tag: AtomicU8::new(TaskTag::Held as u8),
                        pies: SpinLock::new(Vec::new()),
                        heir: SpinLock::new(Vec::new()),
                    },
                    alloc,
                )
            )
            .map_err(|_| MapError::OutOfMemory)?;
            let (ptr, _alloc) = Arc::into_raw_with_allocator(task);
            Arc::from_raw(ptr)
        };
        scheduler::core::enlist(id, &task);
        self.team.push_task(&task);
        self.team.hold(&task);
        conductor::push();
        trace::note(EventKind::Room(RoomEvent::Spawn { tid: id.get() }));
        Ok(task)
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.count_vanished();
    }
}