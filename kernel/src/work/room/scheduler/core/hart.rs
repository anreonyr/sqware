use alloc::sync::Arc;

use crate::lock::{Level, SpinLock};
use crate::memory::manager::addr::PhysAddr;
use crate::runtime::chrono::timer;
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::unit::task::{Task, TaskIdent, TaskState};

use super::ident::Badge;

const QUANTUM_TICKS: u32 = 8;

#[repr(align(64))]
pub(crate) struct Scheduler {
    pub(super) hart: crate::hart::HartId,
    pub(super) inner: SpinLock<SchedulerInner>,
    pub(super) badge: Badge,
}

pub(super) struct SchedulerInner {
    pub(super) running: Option<Arc<Task>>,
    head: Option<Arc<Task>>,
    tail: Option<Arc<Task>>,
}

impl SchedulerInner {
    fn starved_is_empty(&self) -> bool {
        self.head.is_none()
    }
}

impl Scheduler {
    pub(in super::super) fn new(hart: crate::hart::HartId) -> Scheduler {
        Scheduler {
            hart,
            inner: SpinLock::new_level(
                Level::Scheduler,
                SchedulerInner {
                    running: None,
                    head: None,
                    tail: None,
                },
            ),
            badge: Badge::new(),
        }
    }

    fn starved_push(&self, i: &mut SchedulerInner, mut task: Arc<Task>) {
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Starved { next: None }
            ),
            "starved 容器只收 Starved 任务，且入队前不得挂在链上"
        );
        match i.tail.take() {
            None => i.head = Some(task.clone()),
            Some(mut last) => *Task::starved_next(&mut last) = Some(task.clone()),
        }
        i.tail = Some(task);
    }

    fn starved_pop(&self, i: &mut SchedulerInner) -> Option<Arc<Task>> {
        let mut head = i.head.take()?;
        i.head = Task::starved_next(&mut head).take();
        if i.head.is_none() {
            i.tail = None;
        }
        Some(head)
    }

    pub(super) fn starved_remove(&self, i: &mut SchedulerInner, target: &Arc<Task>) -> bool {
        let mut prev: Option<Arc<Task>> = None;
        let mut cur = i.head.clone();
        while let Some(mut node) = cur {
            if Arc::ptr_eq(&node, target) {
                let next = Task::starved_next(&mut node).take();
                let was_tail = next.is_none();
                match &mut prev {
                    Some(p) => *Task::starved_next(p) = next,
                    None => i.head = next,
                }
                if was_tail {
                    i.tail = prev;
                }
                return true;
            }
            prev = Some(node.clone());
            cur = Task::starved_next(&mut node).clone();
        }
        false
    }

    pub(super) fn starved_clear(&self, i: &mut SchedulerInner) {
        while self.starved_pop(i).is_some() {}
    }

    pub(crate) fn push(&self, mut task: Arc<Task>) {
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Starved { .. }
            ),
            "starved 容器只收 Starved 任务"
        );
        let mut i = self.inner.lock();
        self.starved_push(&mut i, task);
    }

    pub(super) fn pull(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.lock();
        self.starved_pop(&mut i)
    }

    fn prepare(&self, task: &mut Arc<Task>) {
        let t = Task::exclusive(task);
        t.transform(TaskState::Running {
            ticks_left: QUANTUM_TICKS,
        });
        // SAFETY: 帧 PA 恒等映射可写；帧属 task 独占
        unsafe {
            let frame = &mut *(frame_pa(&t.ident).as_usize() as *mut TrapContext);
            frame.kernel_sp = trap_stack_edge(self.hart);
            if t.ident.team.space.asid().is_kernel() {
                frame
                    .gpr
                    .set_x(Gprs::TP, crate::hart::per_hart_ptr(self.hart));
            }
        }
        timer::beat_until(timer::blind_ceiling());
    }

    pub(super) fn seat(&self, mut task: Arc<Task>) -> usize {
        let mut i = self.inner.lock();
        self.prepare(&mut task);
        let pa = frame_pa(&task.ident).as_usize();
        self.badge.seat(&task.ident);
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Running { .. }
            ),
            "running 容器只装 Running 任务"
        );
        let prev = i.running.replace(task);
        debug_assert!(prev.is_none(), "装槽前 running 必须为空");
        pa
    }

    pub(crate) fn swap(&self) -> (Arc<Task>, Option<usize>) {
        let mut i = self.inner.lock();
        let task = i.running.take().expect("no running task");
        let next = self.starved_pop(&mut i);
        drop(i);
        let next_pa = if let Some(next) = next {
            let pa = frame_pa(&next.ident).as_usize();
            self.seat(next);
            Some(pa)
        } else {
            self.badge.shed(&task.ident);
            None
        };
        (task, next_pa)
    }

    pub(crate) fn running_task(&self) -> Option<Arc<Task>> {
        let i = self.inner.lock();
        i.running.as_ref().map(Arc::clone)
    }

    fn rotate(&self, i: &mut SchedulerInner, mut cur: Arc<Task>) -> Arc<Task> {
        Task::exclusive(&mut cur).transform(TaskState::Starved { next: None });
        self.starved_push(i, cur);
        self.starved_pop(i).expect("non-empty")
    }

    pub(crate) fn starve(&self) -> usize {
        let mut i = self.inner.lock();
        let Some(cur) = i.running.take() else {
            panic!("starve with no running task on hart {}", self.hart);
        };
        if i.starved_is_empty() {
            let pa = frame_pa(&cur.ident).as_usize();
            i.running = Some(cur);
            return pa;
        }
        let prev_tid = cur.ident.id.get();
        let next = self.rotate(&mut i, cur);
        drop(i);
        let pa = self.seat(next);
        trace::note(EventKind::Room(RoomEvent::Starve { tid: prev_tid }));
        pa
    }

    pub(in super::super) fn advance(&self) -> Option<usize> {
        let mut i = self.inner.lock();
        let mut cur = i.running.take()?;
        let ticks_left = match Task::exclusive(&mut cur).state() {
            TaskState::Running { ticks_left } => *ticks_left,
            _ => unreachable!("running 容器里不是 Running 任务"),
        };
        if ticks_left > 1 || i.starved_is_empty() {
            if ticks_left > 1 {
                Task::exclusive(&mut cur).dec_ticks_left();
            }
            let pa = frame_pa(&cur.ident).as_usize();
            i.running = Some(cur);
            return Some(pa);
        }
        let prev_tid = cur.ident.id.get();
        let next = self.rotate(&mut i, cur);
        let next_tid = next.ident.id.get();
        drop(i);
        let pa = self.seat(next);
        trace::note(EventKind::Room(RoomEvent::Switch { prev_tid, next_tid }));
        Some(pa)
    }
}

pub(super) fn frame_pa(ident: &TaskIdent) -> PhysAddr {
    ident.frame.pa.expect("frame span has pa")
}
