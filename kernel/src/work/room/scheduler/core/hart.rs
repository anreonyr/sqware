use alloc::sync::Arc;
use core::time::Duration;

use crate::lock::{Level, SpinLock};
use crate::memory::manager::addr::PhysAddr;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::unit::task::{Task, TaskIdent, TaskState};

use super::ident::Badge;

const QUANTUM_TICKS: u32 = 8;
const READY_MS: u64 = 2;

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
    ready_since: Option<u64>,
}

impl SchedulerInner {
    fn ready_ceiling(&self) -> u64 {
        self.ready_since.map_or(timer::blind_ceiling(), |since| {
            let budget = clock::duration_to_ticks(Duration::from_millis(READY_MS));
            budget.saturating_sub(clock::now().as_ticks().saturating_sub(since))
        })
    }

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
                    ready_since: None,
                },
            ),
            badge: Badge::new(),
        }
    }

    fn starved_push(&self, i: &mut SchedulerInner, mut task: Arc<Task>) {
        debug_assert!(
            matches!(&*task.state(), TaskState::Starved { next: None }),
            "starved 容器只收 Starved 任务，且入队前不得挂在链上"
        );
        match i.tail.take() {
            None => {
                i.ready_since = Some(clock::now().as_ticks());
                i.head = Some(task.clone());
            }
            Some(last) => Task::set_starved_next(&last, Some(task.clone())),
        }
        i.tail = Some(task);
    }

    fn starved_pop(&self, i: &mut SchedulerInner) -> Option<Arc<Task>> {
        loop {
            let head = i.head.take()?;
            i.head = Task::take_starved_next(&head);
            if i.head.is_none() {
                i.tail = None;
                i.ready_since = None;
            } else {
                i.ready_since = Some(clock::now().as_ticks());
            }
            if head.stopped() || head.ident.team.paused() {
                Task::park(head);
                continue;
            }
            return Some(head);
        }
    }

    pub(super) fn starved_remove(&self, i: &mut SchedulerInner, target: &Arc<Task>) -> bool {
        let mut prev: Option<Arc<Task>> = None;
        let mut cur = i.head.clone();
        while let Some(node) = cur {
            if Arc::ptr_eq(&node, target) {
                let next = Task::take_starved_next(&node);
                let was_tail = next.is_none();
                match &mut prev {
                    Some(p) => Task::set_starved_next(p, next),
                    None => i.head = next,
                }
                if was_tail {
                    i.tail = prev;
                }
                if i.head.is_none() {
                    i.ready_since = None;
                }
                return true;
            }
            prev = Some(node.clone());
            cur = Task::starved_next(&node);
        }
        false
    }

    pub(super) fn starved_clear(&self, i: &mut SchedulerInner) {
        let _commit = crate::work::unit::commit();
        while self.starved_pop(i).is_some() {}
    }

    pub(crate) fn push(&self, mut task: Arc<Task>) -> bool {
        let _commit = crate::work::unit::commit();
        debug_assert!(
            matches!(&*task.state(), TaskState::Starved { .. }),
            "starved 容器只收 Starved 任务"
        );
        let mut i = self.inner.lock();
        let notify = i.starved_is_empty();
        self.starved_push(&mut i, task);
        notify
    }

    pub(super) fn pull(&self) -> Option<Arc<Task>> {
        let _commit = crate::work::unit::commit();
        let mut i = self.inner.lock();
        self.starved_pop(&mut i)
    }

    pub(super) fn steal(&self) -> Option<Arc<Task>> {
        let _commit = crate::work::unit::commit();
        let mut i = self.inner.try_lock()?;
        self.starved_pop(&mut i)
    }

    fn prepare(&self, task: &mut Arc<Task>, ceiling: u64) {
        let t = task.as_ref();
        t.transform(TaskState::Running {
            hart: self.hart,
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
        timer::beat_until(ceiling);
    }

    pub(super) fn seat(&self, mut task: Arc<Task>) -> Option<usize> {
        let _commit = crate::work::unit::commit();
        let mut i = self.inner.lock();
        if task.stopped() || task.ident.team.paused() {
            Task::park(task);
            return None;
        }
        self.prepare(&mut task, i.ready_ceiling());
        let pa = frame_pa(&task.ident).as_usize();
        self.badge.seat(&task.ident);
        debug_assert!(
            matches!(&*task.state(), TaskState::Running { .. }),
            "running 容器只装 Running 任务"
        );
        let prev = i.running.replace(task);
        debug_assert!(prev.is_none(), "装槽前 running 必须为空");
        Some(pa)
    }

    pub(crate) fn swap(&self) -> (Arc<Task>, Option<usize>) {
        let _commit = crate::work::unit::commit();
        let mut i = self.inner.lock();
        let task = i.running.take().expect("no running task");
        let next = self.starved_pop(&mut i);
        drop(i);
        self.badge.shed(&task.ident);
        let next_pa = if let Some(next) = next {
            self.seat(next)
        } else {
            None
        };
        (task, next_pa)
    }

    pub(crate) fn running_task(&self) -> Option<Arc<Task>> {
        let i = self.inner.lock();
        i.running.as_ref().map(Arc::clone)
    }

    fn rotate(&self, i: &mut SchedulerInner, mut cur: Arc<Task>) -> Option<Arc<Task>> {
        cur.transform(TaskState::Starved { next: None });
        self.starved_push(i, cur);
        self.starved_pop(i)
    }

    pub(crate) fn starve(&self) -> usize {
        let _commit = crate::work::unit::commit();
        if self
            .running_task()
            .is_some_and(|t| matches!(t.tag(), crate::work::unit::task::TaskTag::Doomed))
        {
            drop(_commit);
            return crate::work::room::messenger::quit();
        }
        if self
            .running_task()
            .is_some_and(|t| t.stopped() || t.ident.team.paused())
        {
            let next = self.advance();
            drop(_commit);
            return next.unwrap_or_else(super::fetch::fetch);
        }
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
        self.badge.shed(&cur.ident);
        let next = self.rotate(&mut i, cur);
        drop(i);
        let pa = next.and_then(|next| self.seat(next));
        trace::note(EventKind::Room(RoomEvent::Starve { tid: prev_tid }));
        drop(_commit);
        pa.unwrap_or_else(super::fetch::fetch)
    }

    pub(in super::super) fn advance(&self) -> Option<usize> {
        let _commit = crate::work::unit::commit();
        let mut i = self.inner.lock();
        let mut cur = i.running.take()?;
        if cur.stopped() || cur.ident.team.paused() {
            self.badge.shed(&cur.ident);
            Task::park(cur);
            let next = self.starved_pop(&mut i);
            drop(i);
            return next.and_then(|next| self.seat(next));
        }
        let ticks_left = match &*cur.state() {
            TaskState::Running { ticks_left, .. } => *ticks_left,
            _ => unreachable!("running 容器里不是 Running 任务"),
        };
        let ceiling = i.ready_ceiling();
        if i.starved_is_empty() || (ticks_left > 1 && ceiling != 0) {
            if ticks_left > 1 {
                cur.dec_ticks_left();
            }
            let pa = frame_pa(&cur.ident).as_usize();
            i.running = Some(cur);
            timer::beat_until(ceiling);
            return Some(pa);
        }
        let prev_tid = cur.ident.id.get();
        self.badge.shed(&cur.ident);
        let next = self.rotate(&mut i, cur);
        let next_tid = next.as_ref().map_or(0, |t| t.ident.id.get());
        drop(i);
        let pa = next.and_then(|next| self.seat(next));
        trace::note(EventKind::Room(RoomEvent::Switch { prev_tid, next_tid }));
        pa
    }
}

pub(super) fn frame_pa(ident: &TaskIdent) -> PhysAddr {
    ident.frame.pa.expect("frame span has pa")
}

#[cfg(debug_assertions)]
#[path = "tests.rs"]
pub mod tests;
