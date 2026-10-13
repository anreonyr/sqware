use alloc::sync::{Arc, Weak};
use alloc::{collections::VecDeque, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};
use crate::runtime::chrono::clock;

use env::{HoleLimits, MailCondition, TaskId};

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::life::Life;
use env::MailFail;

enum Pending {
    Queue(Queue),
    Rung { count: usize, spare: VecDeque<Slot> },
    Dead,
}

#[derive(Default)]
struct Queue {
    hands: VecDeque<Slot>,
    taking: bool,
    bytes: usize,
}

const RING_CAP: usize = 4;

struct Hand {
    from: TaskId,
    buf: Arc<Vec<u8>>,
    at: u64,
}

enum Slot {
    Reserved(usize, usize),
    Ready(Hand),
}
impl Slot {
    fn hand(&self) -> Option<&Hand> {
        match self {
            Self::Ready(hand) => Some(hand),
            Self::Reserved(..) => None,
        }
    }
}

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

    pub(crate) fn try_new(owner: TaskId) -> Result<Arc<Self>, crate::memory::manager::MapError> {
        Self::try_new_with_limits(owner, HoleLimits::default())
    }

    pub(crate) fn try_new_with_limits(
        owner: TaskId,
        limits: HoleLimits,
    ) -> Result<Arc<Self>, crate::memory::manager::MapError> {
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
        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
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

    pub(crate) fn ready(&self, dir: MailCondition) -> bool {
        let pending = self.pending.lock();
        match dir {
            MailCondition::Pull => match &*pending {
                Pending::Queue(q) => q.hands.front().and_then(Slot::hand).is_some() && !q.taking,
                Pending::Rung { count, .. } => *count > 0,
                Pending::Dead => false,
            },
            MailCondition::Signal(_) => false,
            MailCondition::Push => {
                matches!(&*pending, Pending::Queue(q) if q.hands.len() < self.limits.max_messages && q.bytes < self.limits.max_bytes)
            }
            MailCondition::Empty => {
                matches!(&*pending, Pending::Queue(q) if q.hands.is_empty() && !q.taking)
            }
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        seal(self);
    }
}

pub(crate) fn key(meta: &HoleMeta, dir: MailCondition) -> WakeKey {
    WakeKey::Hole { hole: meta.id, dir }
}

pub(crate) fn reserve_len(meta: &HoleMeta, len: usize) -> Result<Reservation<'_>, MailFail> {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let mut pending = meta.pending.lock();
    let q = match &mut *pending {
        Pending::Queue(q) => q,
        Pending::Dead => return Err(MailFail::Dead),
        Pending::Rung { .. } => return Err(MailFail::Busy),
    };
    if len > meta.limits.max_len {
        return Err(MailFail::Denied);
    }
    if q.hands.len() >= meta.limits.max_messages
        || len > meta.limits.max_bytes.saturating_sub(q.bytes)
    {
        return Err(MailFail::Busy);
    }
    q.hands.try_reserve(1).map_err(|_| MailFail::OoM)?;
    let id = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| MailFail::OoM)?;
    q.hands.push_back(Slot::Reserved(id, len));
    q.bytes += len;
    Ok(Reservation { meta, id: Some(id) })
}

pub(crate) struct Reservation<'a> {
    meta: &'a HoleMeta,
    id: Option<usize>,
}
impl Reservation<'_> {
    pub(crate) fn commit(mut self, buf: Arc<Vec<u8>>, from: TaskId) -> Result<(), MailFail> {
        if buf.is_empty() {
            return Err(MailFail::Denied);
        }
        let at = clock::uptime_ticks();
        {
            let mut pending = self.meta.pending.lock();
            let Pending::Queue(q) = &mut *pending else {
                return Err(MailFail::Dead);
            };
            let at_slot = q
                .hands
                .iter()
                .position(|slot| matches!(slot, Slot::Reserved(id, _) if Some(*id) == self.id))
                .ok_or(MailFail::Dead)?;
            let reserved_len = match q.hands[at_slot] {
                Slot::Reserved(_, len) => len,
                _ => unreachable!(),
            };
            if buf.len() > self.meta.limits.max_len {
                return Err(MailFail::Denied);
            }
            if reserved_len != 0 && reserved_len != buf.len() {
                return Err(MailFail::Denied);
            }
            let next_bytes = q.bytes - reserved_len;
            if buf.len() > self.meta.limits.max_bytes.saturating_sub(next_bytes) {
                return Err(MailFail::Busy);
            }
            q.bytes = next_bytes + buf.len();
            q.hands[at_slot] = Slot::Ready(Hand { from, buf, at });
            HANDS_LIVE.fetch_add(1, Ordering::Relaxed);
            self.id = None;
        }
        let _ = messenger::wake(key(self.meta, MailCondition::Pull), &self.meta.life());
        Ok(())
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let removed = {
            let mut pending = self.meta.pending.lock();
            match &mut *pending {
                Pending::Queue(q) => {
                    let at = q
                        .hands
                        .iter()
                        .position(|slot| matches!(slot, Slot::Reserved(known, _) if *known == id));
                    if let Some(Slot::Reserved(_, len)) = at.and_then(|at| q.hands.remove(at)) {
                        q.bytes -= len;
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        };
        if removed {
            let _ = messenger::wake(key(self.meta, MailCondition::Pull), &self.meta.life());
            let _ = messenger::wake(key(self.meta, MailCondition::Empty), &self.meta.life());
            let _ = messenger::wake(key(self.meta, MailCondition::Push), &self.meta.life());
        }
    }
}

pub(crate) struct Reading<'a> {
    meta: &'a HoleMeta,
    pub(crate) from: TaskId,
    pub(crate) bytes: Arc<Vec<u8>>,
    finished: bool,
}
impl Reading<'_> {
    pub(crate) fn finish(mut self) {
        taken(self.meta);
        self.finished = true;
    }
}
impl Drop for Reading<'_> {
    fn drop(&mut self) {
        if !self.finished {
            back(self.meta);
        }
    }
}
pub(crate) fn read(meta: &HoleMeta) -> Result<Reading<'_>, MailFail> {
    let mut pending = meta.pending.lock();
    let queue = match &mut *pending {
        Pending::Queue(queue) => queue,
        Pending::Dead => return Err(MailFail::Dead),
        _ => return Err(MailFail::Busy),
    };
    if queue.taking {
        return Err(MailFail::Busy);
    }
    let head = queue
        .hands
        .front()
        .and_then(Slot::hand)
        .ok_or(MailFail::Busy)?;
    let reading = Reading {
        meta,
        from: head.from,
        bytes: head.buf.clone(),
        finished: false,
    };
    queue.taking = true;
    Ok(reading)
}

fn taken(meta: &HoleMeta) {
    let mut off = None;
    {
        let mut pending = meta.pending.lock();
        if let Pending::Queue(q) = &mut *pending {
            if q.taking {
                q.taking = false;
                off = q.hands.pop_front().and_then(|slot| match slot {
                    Slot::Ready(hand) => {
                        q.bytes -= hand.buf.len();
                        Some(hand)
                    }
                    Slot::Reserved(_, len) => {
                        q.bytes -= len;
                        None
                    }
                });
            }
        }
    }
    if let Some(hand) = off {
        let len = hand.buf.len();
        HANDS_LIVE.fetch_sub(1, Ordering::Relaxed);
        note_hand_off(meta, hand.from, len, hand.at);
    }
    let _ = messenger::wake(key(meta, MailCondition::Empty), &meta.life());
    let _ = messenger::wake(key(meta, MailCondition::Push), &meta.life());
    let _ = messenger::wake(key(meta, MailCondition::Pull), &meta.life());
}

fn back(meta: &HoleMeta) {
    {
        let mut pending = meta.pending.lock();
        if let Pending::Queue(q) = &mut *pending {
            q.taking = false;
        }
    }
    let _ = messenger::wake(key(meta, MailCondition::Pull), &meta.life());
}

pub(crate) fn peek(meta: &HoleMeta) -> Result<(usize, TaskId, usize), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Queue(q) => match q.hands.front().and_then(Slot::hand) {
            Some(hand) => Ok((
                hand.buf.len(),
                hand.from,
                q.hands.iter().filter_map(Slot::hand).count(),
            )),
            None => Err(MailFail::Busy),
        },
        _ => Err(MailFail::Busy),
    }
}

pub(crate) fn ring(meta: &HoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    {
        let mut pending = meta.pending.lock();
        match &mut *pending {
            Pending::Queue(q) if q.hands.is_empty() && !q.taking => {
                *pending = Pending::Rung {
                    count: 1,
                    spare: core::mem::take(&mut q.hands),
                };
            }
            Pending::Rung { count, .. } if *count < RING_CAP => {
                *count += 1;
            }
            _ => return Err(MailFail::Busy),
        }
    };
    let _ = messenger::wake(key(meta, MailCondition::Pull), &meta.life());
    Ok(())
}

pub(crate) fn hush(meta: &HoleMeta) -> Result<(), MailFail> {
    let mut pending = meta.pending.lock();
    match &mut *pending {
        Pending::Rung { count, .. } if *count > 1 => *count -= 1,
        Pending::Rung { spare, .. } => {
            *pending = Pending::Queue(Queue {
                hands: core::mem::take(spare),
                taking: false,
                bytes: 0,
            });
        }
        _ => return Err(MailFail::Busy),
    }
    drop(pending);
    let _ = messenger::wake(key(meta, MailCondition::Push), &meta.life());
    let _ = messenger::wake(key(meta, MailCondition::Empty), &meta.life());
    Ok(())
}

pub(crate) fn wait(
    meta: &HoleMeta,
    dir: MailCondition,
    dur: Duration,
) -> Result<Handoff<bool>, MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if meta.ready(dir) {
        return Ok(Handoff::Resume(true));
    }
    if dur != Duration::ZERO && dir == MailCondition::Empty && hand_age(meta).is_none() {
        note_shape(meta);
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    let out = messenger::wait(key(meta, dir), meta.life(), dur)?;
    if dir == MailCondition::Empty
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
            } else if q.hands.front().and_then(Slot::hand).is_none() {
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
        Pending::Queue(q) => match q.hands.front().and_then(Slot::hand) {
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
    messenger::wipe(key(meta, MailCondition::Pull));
    messenger::wipe(key(meta, MailCondition::Empty));
    messenger::wipe(key(meta, MailCondition::Push));
}

#[cfg(debug_assertions)]
pub mod tests;
