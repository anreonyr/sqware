//! 调度器：跨核的那一面 —— 全局表、在世任务名册、放置与拆除。
//!
//! 每核一张运行槽（`conductor::Conductor`）住这里那张表里。放活、偷活、按名找
//! 任务、停机拆台都得先过这张表；哪几核空着、往哪放、怎么叫醒它们，在 `idle`。

pub(crate) mod idle;
pub(crate) mod roster;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::core::memory::nest::NestError;
use crate::core::work::room::conductor::Conductor;
use crate::core::work::room::counter::Counter;
use crate::core::work::room::messenger;
use crate::core::work::unit::task::Task;
use crate::machine::hart::{self, HartId};
use crate::machine::sync::OnceLock;

use self::idle::{nudge, pick, waiting};
use self::roster::roster;

/// 每核一张的运行槽，按 hart 号排。
static SCHEDULERS: OnceLock<&'static [Conductor]> = OnceLock::new();

static STEAL_CURSOR: AtomicUsize = AtomicUsize::new(0);

static KICKS: Counter = Counter::new();
static FALLBACK: Counter = Counter::new();

/// 装好全局表：每核一张空槽，并把本核的槽挂进 `PerHart`。
pub(crate) fn init() -> Result<(), NestError> {
    let n = hart::hart_count();
    assert!(n > 0, "no harts");
    let mut sched =
        Box::<[Conductor]>::try_new_uninit_slice(n).map_err(|_| NestError::OutOfMemory)?;
    for (h, slot) in sched.iter_mut().enumerate() {
        slot.write(Conductor::new(HartId::new(h)));
    }
    let mut sched = unsafe { sched.assume_init() };
    for (h, c) in sched.iter_mut().enumerate() {
        hart::set_scheduler(HartId::new(h), c as *mut Conductor as *mut ());
    }
    assert!(
        SCHEDULERS.set(Box::leak(sched)).is_ok(),
        "schedulers double init"
    );
    Ok(())
}

/// 表装好了没有 —— 装之前本核还没有运行槽，问身份也就只能答“没有”。
pub(crate) fn installed() -> bool {
    SCHEDULERS.get().is_some()
}

fn schedulers() -> &'static [Conductor] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

#[cfg(debug_assertions)]
pub(crate) fn scheduler_addr(i: HartId) -> usize {
    core::ptr::addr_of!(schedulers()[i.get()]) as usize
}

/// 把一枚就绪任务放到挑中的那颗核上。
pub(crate) fn launch(task: Arc<Task>) {
    kick(pick(), task);
}

/// 把一枚就绪任务放到指定核上：接进它那张饥饿队列，必要时叫醒它。
pub(crate) fn kick(hart: HartId, task: Arc<Task>) {
    let notify = schedulers()[hart.get()].push(task);
    if waiting(hart) || notify {
        notify_hart(hart);
        if !waiting(hart)
            && let Some(idle) = schedulers()
                .iter()
                .find(|s| s.hart != hart && waiting(s.hart))
        {
            notify_hart(idle.hart);
        }
    } else {
        FALLBACK.bump();
    }
}

fn notify_hart(hart: HartId) {
    KICKS.bump();
    nudge(hart);
}

/// 从别的核的饥饿队列里偷一枚（本核自己那张由 `pull` 管）。
pub(crate) fn steal() -> Option<Arc<Task>> {
    let all = schedulers();
    let me = hart::hart_id();
    let start = STEAL_CURSOR.fetch_add(1, Ordering::Relaxed) % all.len();
    for offset in 0..all.len() {
        let victim = &all[(start + offset) % all.len()];
        if victim.hart != me
            && let Some(task) = victim.steal()
        {
            return Some(task);
        }
    }
    None
}

/// 把一枚任务从任何一张饥饿队列里摘掉。
pub(crate) fn remove_from_starved(target: &Arc<Task>) -> bool {
    let _commit = crate::core::work::commit();
    for s in schedulers() {
        let mut i = s.inner.lock();
        if s.starved_remove(&mut i, target) {
            drop(i);
            return true;
        }
    }
    false
}

/// 停机拆台：清空每张饥饿队列，交还名册里还挂着的东西，再拆掉等待面。
pub(crate) fn rip() {
    let _commit = crate::core::work::commit();
    let Some(cs) = SCHEDULERS.get() else { return };
    for c in cs.iter() {
        let mut i = c.inner.lock();
        c.starved_clear(&mut i);
    }
    for task in roster().into_iter().filter_map(|w| w.upgrade()) {
        task.ident.team.release_held(&task);
    }
    messenger::rip();
    if let Some(r) = roster::ROSTER.get() {
        r.lock().clear();
    }
}

/// kick 的读数：走了核间中断几次 / 不必叫的几次。
pub(crate) fn kick_stats() -> (usize, usize) {
    (KICKS.get(), FALLBACK.get())
}
