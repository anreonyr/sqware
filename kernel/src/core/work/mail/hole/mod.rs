mod hand;
mod pending;
mod queue;
mod reading;
mod reservation;
mod slot;

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;

use crate::machine::clock;
use crate::machine::sync::{Level, SpinLock};

use env::{HoleCondition, HoleFail, HoleLimits, TaskId};

use crate::core::work::pend::Need;
use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::unit::life::Life;

use pending::Pending;
use queue::Queue;
use slot::Slot;

pub(crate) use pending::{hush, ring};
pub(crate) use reading::{peek, read};
pub(crate) use reservation::reserve_len;

pub struct HoleMeta {
    pub(crate) id: usize,
    life: Arc<Life>,
    owner: TaskId,
    pending: SpinLock<Pending>,
    limits: HoleLimits,
    alarmed: AtomicBool,
    stuck: AtomicBool,
}

impl HoleMeta {
    #[cfg(debug_assertions)]
    pub(crate) fn new(owner: TaskId) -> Arc<Self> {
        Self::try_new(owner).expect("hole allocation failed")
    }

    pub(crate) fn try_new(
        owner: TaskId,
    ) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        Self::try_new_with_limits(owner, HoleLimits::default())
    }

    pub(crate) fn try_new_with_limits(
        owner: TaskId,
        limits: HoleLimits,
    ) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        let life = Life::try_new()?;
        Arc::try_new(Self {
            id: super::next_id(),
            life,
            owner,
            pending: SpinLock::new_level(Level::L3, Pending::Queue(Queue::default())),
            limits,
            alarmed: AtomicBool::new(false),
            stuck: AtomicBool::new(false),
        })
        .map_err(|_| crate::core::memory::nest::NestError::OutOfMemory)
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn alive(&self) -> bool {
        !matches!(*self.pending.lock(), Pending::Dead)
    }

    /// 这一种等待此刻是否已就绪：队列按队头与库存，计数通知按计数。
    pub(crate) fn ready(&self, dir: HoleCondition) -> bool {
        let pending = self.pending.lock();
        match &*pending {
            Pending::Queue(queue) => queue.ready(dir, &self.limits),
            Pending::Rung { count, .. } => {
                matches!(dir, HoleCondition::Pull | HoleCondition::Signal) && *count > 0
            }
            Pending::Dead => false,
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        seal(self);
    }
}

pub(crate) fn key(meta: &HoleMeta, dir: HoleCondition) -> WakeKey {
    WakeKey::Hole { hole: meta.id, dir }
}

/// 等这一种状态要哪一种许可：取数与信号是 FETCH，放数与空位是 STORE。
pub(crate) fn need(dir: HoleCondition) -> Need {
    match dir {
        HoleCondition::Pull | HoleCondition::Signal => Need::Fetch,
        HoleCondition::Push | HoleCondition::Empty => Need::Store,
    }
}

pub(crate) fn wait(
    meta: &HoleMeta,
    dir: HoleCondition,
    dur: Duration,
) -> Result<Handoff<bool>, HoleFail> {
    if !meta.alive() {
        return Err(HoleFail::Dead);
    }
    if meta.ready(dir) {
        return Ok(Handoff::Resume(true));
    }
    if dur != Duration::ZERO && dir == HoleCondition::Empty && hand_age(meta).is_none() {
        note_shape(meta);
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    let out = messenger::wait(key(meta, dir), meta.life(), dur)?;
    if dir == HoleCondition::Empty
        && let Some((from, len, ms)) = hand_age(meta)
    {
        note_hold(meta, from, ms);
        alarm_stuck(meta, from, len, ms);
    }
    Ok(match out {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

static HOLD_N: AtomicUsize = AtomicUsize::new(0);
static HOLD_MAX_MS: AtomicUsize = AtomicUsize::new(0);
static HOLD_WORST: AtomicUsize = AtomicUsize::new(0);
static HOLD_FROM: AtomicUsize = AtomicUsize::new(0);
static HOLD_OWNER: AtomicUsize = AtomicUsize::new(0);
static HANDS_LIVE: AtomicUsize = AtomicUsize::new(0);
static ALARM_N: AtomicUsize = AtomicUsize::new(0);
const HOLD_MS: usize = 1000;
const ALARM_MAX: usize = 8;

fn note_shape(meta: &HoleMeta) {
    static N: AtomicUsize = AtomicUsize::new(0);
    if meta.stuck.swap(true, Ordering::Relaxed) {
        return;
    }
    N.fetch_add(1, Ordering::Relaxed);
    if N.load(Ordering::Relaxed) > ALARM_MAX {
        return;
    }
    let shape = match &*meta.pending.lock() {
        Pending::Queue(q) => {
            if q.hands.is_empty() {
                "queue-empty"
            } else if q.head().is_none() {
                "queue-reserved"
            } else if q.taking {
                "queue-taking"
            } else {
                "queue-hands"
            }
        }
        Pending::Rung { .. } => "rung",
        Pending::Dead => "dead",
    };
    crate::putln!(
        "mail: push not ready hole#{} owner={} pending={} live={}",
        meta.id,
        meta.owner.get(),
        shape,
        HANDS_LIVE.load(Ordering::Relaxed),
    );
}

fn elapsed_ms(at: u64) -> usize {
    clock::ticks_to_duration(clock::uptime_ticks().wrapping_sub(at)).as_millis() as usize
}

fn hand_age(meta: &HoleMeta) -> Option<(TaskId, usize, usize)> {
    let pending = meta.pending.lock();
    let (from, len, at) = match &*pending {
        Pending::Queue(q) => match q.head() {
            Some(hand) => (hand.from, hand.buf.len(), hand.at),
            None => return None,
        },
        _ => return None,
    };
    if at == 0 {
        return None;
    }
    Some((from, len, elapsed_ms(at)))
}

fn note_hold(meta: &HoleMeta, from: TaskId, ms: usize) {
    if ms < HOLD_MS {
        return;
    }
    HOLD_N.fetch_add(1, Ordering::Relaxed);
    if ms > HOLD_MAX_MS.load(Ordering::Relaxed) {
        HOLD_MAX_MS.store(ms, Ordering::Relaxed);
        HOLD_WORST.store(meta.id, Ordering::Relaxed);
        HOLD_FROM.store(from.get(), Ordering::Relaxed);
        HOLD_OWNER.store(meta.owner.get(), Ordering::Relaxed);
    }
}

fn alarm_stuck(meta: &HoleMeta, from: TaskId, len: usize, ms: usize) {
    if ms < HOLD_MS || meta.alarmed.swap(true, Ordering::Relaxed) {
        return;
    }
    if ALARM_N.fetch_add(1, Ordering::Relaxed) >= ALARM_MAX {
        return;
    }
    crate::putln!(
        "mail: hand stuck hole#{} from={} owner={} len={} age={}ms",
        meta.id,
        from.get(),
        meta.owner.get(),
        len,
        ms,
    );
}

fn note_hand_off(meta: &HoleMeta, from: TaskId, len: usize, at: u64) {
    if at == 0 {
        return;
    }
    let ms = elapsed_ms(at);
    note_hold(meta, from, ms);
    alarm_stuck(meta, from, len, ms);
}

pub(crate) fn hold_line() {
    crate::putln!(
        "hole: push_hold_n={} push_hold_max_ms={} worst=hole#{} from={} owner={} live={} back_n={} back_max_len={}",
        HOLD_N.load(Ordering::Relaxed),
        HOLD_MAX_MS.load(Ordering::Relaxed),
        HOLD_WORST.load(Ordering::Relaxed),
        HOLD_FROM.load(Ordering::Relaxed),
        HOLD_OWNER.load(Ordering::Relaxed),
        HANDS_LIVE.load(Ordering::Relaxed),
        BACK_N.load(Ordering::Relaxed),
        BACK_MAX_LEN.load(Ordering::Relaxed),
    );
}

static BACK_N: AtomicUsize = AtomicUsize::new(0);
static BACK_MAX_LEN: AtomicUsize = AtomicUsize::new(0);
static BACK_ALARMED: AtomicBool = AtomicBool::new(false);

pub(crate) fn note_back(len: usize, max: usize) {
    BACK_N.fetch_add(1, Ordering::Relaxed);
    BACK_MAX_LEN.fetch_max(len, Ordering::Relaxed);
    if !BACK_ALARMED.swap(true, Ordering::Relaxed) {
        crate::putln!("mail: hand returned len={} max={}", len, max);
    }
}

pub(crate) fn seal(meta: &HoleMeta) {
    let pending = core::mem::replace(&mut *meta.pending.lock(), Pending::Dead);
    if let Pending::Queue(q) = &pending {
        if let Some(head) = q.hands.iter().filter_map(Slot::hand).next() {
            note_hand_off(meta, head.from, head.buf.len(), head.at);
        }
        HANDS_LIVE.fetch_sub(
            q.hands.iter().filter_map(Slot::hand).count(),
            Ordering::Relaxed,
        );
    }
    drop(pending);
    messenger::wipe(WakeKey::seal(env::PieKind::Hole, meta.id));
    messenger::wipe(key(meta, HoleCondition::Pull));
    messenger::wipe(key(meta, HoleCondition::Empty));
    messenger::wipe(key(meta, HoleCondition::Push));
    messenger::wipe(key(meta, HoleCondition::Signal));
}

#[cfg(debug_assertions)]
#[path = "../../../../../tests/health/hole.rs"]
pub mod tests;
