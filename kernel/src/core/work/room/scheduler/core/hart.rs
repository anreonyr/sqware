use alloc::sync::Arc;
use core::time::Duration;

use crate::core::work::record::trace::{self, EventKind, RoomEvent};
use crate::core::work::room::deadline;
use crate::core::work::room::Chain;
use crate::core::work::unit::task::{Task, TaskState};
use crate::machine::clock;
use crate::machine::switcher::context::{Gprs, TrapContext};
use crate::machine::switcher::stack::trap_stack_edge;
use crate::machine::sync::{Level, SpinLock};

use super::ident::Badge;

const QUANTUM_TICKS: u32 = 8;
const READY_MS: u64 = 2;

#[repr(align(64))]
pub(crate) struct Scheduler {
    pub(super) hart: crate::machine::hart::HartId,
    pub(super) inner: SpinLock<SchedulerInner>,
    pub(super) badge: Badge,
}

pub(super) struct SchedulerInner {
    pub(super) running: Option<Arc<Task>>,
    chain: Chain,
    ready_since: Option<u64>,
}

impl SchedulerInner {
    fn ready_ceiling(&self) -> u64 {
        self.ready_since.map_or(deadline::blind_ceiling(), |since| {
            let budget = clock::duration_to_ticks(Duration::from_millis(READY_MS));
            budget.saturating_sub(clock::now().as_ticks().saturating_sub(since))
        })
    }

    fn starved_is_empty(&self) -> bool {
        self.chain.is_empty()
    }
}

impl Scheduler {
    pub(in super::super) fn new(hart: crate::machine::hart::HartId) -> Scheduler {
        Scheduler {
            hart,
            inner: SpinLock::new_level(
                Level::Scheduler,
                SchedulerInner {
                    running: None,
                    chain: Chain::new(),
                    ready_since: None,
                },
            ),
            badge: Badge::new(),
        }
    }

    fn starved_push(&self, i: &mut SchedulerInner, task: Arc<Task>) {
        debug_assert!(
            task.ready() && task.next().is_none(),
            "starved 容器只收就绪任务，且入队前不得挂在链上"
        );
        if i.chain.is_empty() {
            i.ready_since = Some(clock::now().as_ticks());
        }
        i.chain.push(task);
    }

    fn starved_pop(&self, i: &mut SchedulerInner) -> Option<Arc<Task>> {
        loop {
            let head = i.chain.pop()?;
            i.ready_since = if i.chain.is_empty() {
                None
            } else {
                Some(clock::now().as_ticks())
            };
            if head.stopped() || head.ident.team.paused() {
                Task::park(head);
                continue;
            }
            return Some(head);
        }
    }

    pub(super) fn starved_remove(&self, i: &mut SchedulerInner, target: &Arc<Task>) -> bool {
        let removed = i.chain.remove_if(&mut |t| Arc::ptr_eq(t, target)).is_some();
        if removed && i.chain.is_empty() {
            i.ready_since = None;
        }
        removed
    }

    pub(super) fn starved_clear(&self, i: &mut SchedulerInner) {
        let _commit = crate::core::work::unit::commit();
        while self.starved_pop(i).is_some() {}
    }

    pub(crate) fn push(&self, task: Arc<Task>) -> bool {
        let _commit = crate::core::work::unit::commit();
        debug_assert!(task.ready(), "starved 容器只收就绪任务");
        let mut i = self.inner.lock();
        let notify = i.starved_is_empty();
        self.starved_push(&mut i, task);
        notify
    }

    pub(super) fn pull(&self) -> Option<Arc<Task>> {
        let _commit = crate::core::work::unit::commit();
        let mut i = self.inner.lock();
        self.starved_pop(&mut i)
    }

    pub(super) fn steal(&self) -> Option<Arc<Task>> {
        let _commit = crate::core::work::unit::commit();
        let mut i = self.inner.try_lock()?;
        self.starved_pop(&mut i)
    }

    fn prepare(&self, task: &mut Arc<Task>, ceiling: u64) {
        let t = task.as_ref();
        t.transform_checked(TaskState::running(self.hart, QUANTUM_TICKS));
        unsafe {
            let frame = &mut *(t.ident.frame_pa.as_usize() as *mut TrapContext);
            frame.kernel_sp = trap_stack_edge(self.hart);
            if t.ident.team.space.tree.asid().is_kernel() {
                frame
                    .gpr
                    .set_x(Gprs::TP, crate::machine::hart::per_hart_ptr(self.hart));
            }
        }
        deadline::beat_until(ceiling);
    }

    pub(super) fn seat(&self, mut task: Arc<Task>) -> Option<usize> {
        let _commit = crate::core::work::unit::commit();
        let mut i = self.inner.lock();
        if task.stopped() || task.ident.team.paused() {
            Task::park(task);
            return None;
        }
        self.prepare(&mut task, i.ready_ceiling());
        let pa = task.ident.frame_pa.as_usize();
        self.badge.seat(&task.ident);
        debug_assert!(task.on_hart().is_some(), "running 容器只装在核上的任务");
        let prev = i.running.replace(task);
        debug_assert!(prev.is_none(), "装槽前 running 必须为空");
        Some(pa)
    }

    pub(crate) fn swap(&self) -> (Arc<Task>, Option<usize>) {
        let _commit = crate::core::work::unit::commit();
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

    fn rotate(&self, i: &mut SchedulerInner, cur: Arc<Task>) -> Option<Arc<Task>> {
        cur.transform_checked(TaskState::ready());
        self.starved_push(i, cur);
        self.starved_pop(i)
    }

    pub(crate) fn starve(&self) -> usize {
        let _commit = crate::core::work::unit::commit();
        if self.running_task().is_some_and(|t| t.doomed()) {
            drop(_commit);
            return crate::core::work::room::messenger::quit();
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
            let pa = cur.ident.frame_pa.as_usize();
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
        let _commit = crate::core::work::unit::commit();
        let mut i = self.inner.lock();
        let cur = i.running.take()?;
        if cur.stopped() || cur.ident.team.paused() {
            self.badge.shed(&cur.ident);
            Task::park(cur);
            let next = self.starved_pop(&mut i);
            drop(i);
            return next.and_then(|next| self.seat(next));
        }
        let ticks_left = cur.ticks_left();
        let ceiling = i.ready_ceiling();
        if i.starved_is_empty() || (ticks_left > 1 && ceiling != 0) {
            if ticks_left > 1 {
                cur.dec_ticks_left();
            }
            let pa = cur.ident.frame_pa.as_usize();
            i.running = Some(cur);
            deadline::beat_until(ceiling);
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

#[cfg(debug_assertions)]
#[path = "../../../../../../tests/health/scheduler.rs"]
pub mod tests;
