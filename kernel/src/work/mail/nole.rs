use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use env::TaskId;

use crate::lock::{Level, SpinLock};

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::life::Life;
use env::MailFail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NoleId(pub usize);

fn alloc_id() -> NoleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    NoleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoleState {
    Live,
    Dead,
}

pub struct NoleMeta {
    state: SpinLock<NoleState>,
    id: NoleId,
    life: Arc<Life>,
    ring: SpinLock<bool>,
    owner: TaskId,
}

impl NoleMeta {
    #[cfg(debug_assertions)]
    pub(crate) fn new(owner: TaskId) -> Arc<Self> {
        Self::try_new(owner).expect("nole allocation failed")
    }

    pub(crate) fn try_new(owner: TaskId) -> Result<Arc<Self>, crate::memory::manager::MapError> {
        let life = Life::try_new()?;
        Arc::try_new(Self {
            state: SpinLock::new_level(Level::L3, NoleState::Live),
            id: alloc_id(),
            life,
            ring: SpinLock::new_level(Level::L3, false),
            owner,
        })
        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn id(&self) -> NoleId {
        self.id
    }

    pub(crate) fn alive(&self) -> bool {
        matches!(*self.state.lock(), NoleState::Live)
    }

    pub(crate) fn ready(&self) -> bool {
        *self.ring.lock()
    }
}

impl Drop for NoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = NoleState::Dead;
        messenger::wipe(WakeKey::Seal {
            kind: env::PieKind::Nole as u8,
            id: self.id.0,
        });
        messenger::wipe(key(self));
    }
}

pub(crate) fn key(meta: &NoleMeta) -> WakeKey {
    WakeKey::Nole { id: meta.id.0 }
}

pub(crate) fn ring(meta: &NoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    {
        let mut ring = meta.ring.lock();
        if *ring {
            return Err(MailFail::Busy);
        }
        *ring = true;
    }
    let _ = messenger::wake(key(meta), &meta.life());
    Ok(())
}

pub(crate) fn hush(meta: &NoleMeta) -> Result<(), MailFail> {
    let mut ring = meta.ring.lock();
    if !*ring {
        return Err(MailFail::Busy);
    }
    *ring = false;
    Ok(())
}

pub(crate) fn wait(meta: &NoleMeta, dur: Duration) -> Result<Handoff<bool>, MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if meta.ready() {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match messenger::wait(key(meta), meta.life(), dur)? {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready()),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

pub(crate) fn seal(meta: &NoleMeta) {
    *meta.state.lock() = NoleState::Dead;
    messenger::wipe(WakeKey::Seal {
        kind: env::PieKind::Nole as u8,
        id: meta.id.0,
    });
    messenger::wipe(key(meta));
}
