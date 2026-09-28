use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};

use env::{HoleDir, TaskId};

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::life::Life;
use env::MailFail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoleId(pub usize);

fn alloc_id() -> HoleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    HoleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleState {
    Live,
    Dead,
}

struct Slot {
    buf: Vec<u8>,
    from: TaskId,
}

pub struct HoleMeta {
    state: SpinLock<HoleState>,
    id: HoleId,
    life: Arc<Life>,
    slot: SpinLock<Slot>,
    owner: TaskId,
}

impl HoleMeta {
    pub(super) fn new(id: HoleId, owner: TaskId) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            id,
            life: Life::new(),
            slot: SpinLock::new_level(
                Level::L3,
                Slot {
                    buf: Vec::new(),
                    from: TaskId::new(0),
                },
            ),
            owner,
        })
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn id(&self) -> HoleId {
        self.id
    }

    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == HoleState::Live
    }

    pub(crate) fn ready(&self, dir: HoleDir) -> bool {
        let slot = self.slot.lock();
        match dir {
            HoleDir::Pull => !slot.buf.is_empty(),
            HoleDir::Push => slot.buf.is_empty(),
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
        messenger::wipe(key(self, HoleDir::Pull));
        messenger::wipe(key(self, HoleDir::Push));
    }
}

pub(crate) fn key(meta: &HoleMeta, dir: HoleDir) -> WakeKey {
    WakeKey::Hole {
        hole: meta.id.0,
        dir,
    }
}

pub(crate) fn try_push(meta: &HoleMeta, msg: &mut Vec<u8>, from: TaskId) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if msg.is_empty() {
        return Err(MailFail::Denied);
    }
    let mut slot = meta.slot.lock();
    if !slot.buf.is_empty() {
        return Err(MailFail::Busy);
    }
    core::mem::swap(&mut slot.buf, msg);
    slot.from = from;
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Pull), &meta.life());
    Ok(())
}

pub(crate) fn peek(meta: &HoleMeta) -> Result<(usize, TaskId), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let slot = meta.slot.lock();
    if slot.buf.is_empty() {
        return Err(MailFail::Busy);
    }
    Ok((slot.buf.len(), slot.from))
}

pub(crate) fn try_pull(meta: &HoleMeta, max: usize) -> Result<(Vec<u8>, TaskId), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let mut slot = meta.slot.lock();
    let len = slot.buf.len();
    if len == 0 {
        return Err(MailFail::Busy);
    }
    if len > max {
        return Err(MailFail::Denied);
    }
    let from = slot.from;
    let msg = core::mem::take(&mut slot.buf);
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
    Ok((msg, from))
}

pub(crate) fn wait(
    meta: &HoleMeta,
    dir: HoleDir,
    dur: Duration,
) -> Result<Handoff<bool>, MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if meta.ready(dir) {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match messenger::wait(key(meta, dir), meta.life(), dur)? {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

pub(crate) fn seal(meta: &HoleMeta) {
    *meta.state.lock() = HoleState::Dead;
    messenger::wipe(key(meta, HoleDir::Pull));
    messenger::wipe(key(meta, HoleDir::Push));
}

pub(crate) fn meta(owner: TaskId) -> Arc<HoleMeta> {
    HoleMeta::new(alloc_id(), owner)
}