use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::lock::{Level, SpinLock};

use env::{HoleDir, TaskId};

use crate::work::mail::hole::HoleId;
use crate::work::mail::nole::NoleId;
use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::life::Life;
use core::time::Duration;
use env::ToleFail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToleId(pub usize);

fn alloc_id() -> ToleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    ToleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToleState {
    Live,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mate {
    Hole(HoleId, HoleDir),
    Nole(NoleId),
}

impl Mate {
    pub(crate) fn key(self) -> WakeKey {
        match self {
            Mate::Hole(id, dir) => WakeKey::Hole { hole: id.0, dir },
            Mate::Nole(id) => WakeKey::Nole { id: id.0 },
        }
    }
}

#[derive(Clone)]
pub(crate) struct Cell {
    mate: Mate,
    life: Weak<Life>,
}

impl Cell {
    pub(crate) fn live(&self) -> bool {
        !Life::dead(&self.life)
    }

    pub(crate) fn mate(&self) -> Mate {
        self.mate
    }
}

pub struct ToleMeta {
    state: SpinLock<ToleState>,
    id: ToleId,
    life: Arc<Life>,
    cells: SpinLock<Vec<Cell>>,
    owner: TaskId,
}

pub(crate) fn key(meta: &ToleMeta) -> WakeKey {
    WakeKey::Tole { id: meta.id().0 }
}

impl ToleMeta {
    fn new(id: ToleId, owner: TaskId) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, ToleState::Live),
            id,
            life: Life::new(),
            cells: SpinLock::new_level(Level::L3, Vec::new()),
            owner,
        })
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn id(&self) -> ToleId {
        self.id
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn alive(&self) -> bool {
        matches!(*self.state.lock(), ToleState::Live)
    }

    pub(crate) fn cells(&self) -> Vec<Cell> {
        let cells = self.cells.lock();
        let mut out = Vec::new();
        if out.try_reserve(cells.len()).is_err() {
            return Vec::new();
        }
        out.extend(cells.iter().filter(|c| c.live()).cloned());
        out
    }
}

pub(crate) fn attach(meta: &ToleMeta, mate: Mate, life: Weak<Life>) -> Result<(), ToleFail> {
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    let cell = Cell {
        mate,
        life: life.clone(),
    };
    {
        let mut cells = meta.cells.lock();
        if cells.iter().any(|c| c.mate == cell.mate) {
            return Ok(());
        }
        if cells.try_reserve(1).is_err() {
            return Err(ToleFail::OoM);
        }
        cells.push(cell);
    }
    if messenger::forward(mate.key(), life, meta.id.0, meta.life()).is_err() {
        let mut cells = meta.cells.lock();
        if let Some(at) = cells.iter().position(|c| c.mate == mate) {
            cells.swap_remove(at);
        }
        return Err(ToleFail::OoM);
    }
    let _ = messenger::knock(key(meta), &meta.life());
    Ok(())
}

pub(crate) fn detach(meta: &ToleMeta, mate: Mate) -> Result<(), ToleFail> {
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    {
        let mut cells = meta.cells.lock();
        if let Some(at) = cells.iter().position(|c| c.mate == mate) {
            cells.swap_remove(at);
        }
    }
    messenger::unforward(mate.key(), meta.id.0);
    Ok(())
}

pub(crate) fn seal(meta: &ToleMeta) {
    *meta.state.lock() = ToleState::Dead;
    retire(meta);
}

fn retire(meta: &ToleMeta) {
    let cells = core::mem::take(&mut *meta.cells.lock());
    for c in &cells {
        messenger::unforward(c.mate.key(), meta.id.0);
    }
    messenger::wipe(key(meta));
}

impl Drop for ToleMeta {
    fn drop(&mut self) {
        let cells = core::mem::take(&mut *self.cells.lock());
        for c in &cells {
            messenger::unforward(c.mate.key(), self.id.0);
        }
        messenger::wipe(key(self));
    }
}

pub(crate) fn wait(meta: &ToleMeta, dur: Duration) -> Result<Handoff<()>, ToleFail> {
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(()));
    }
    messenger::wait(key(meta), meta.life(), dur)
}

pub(crate) fn meta(owner: TaskId) -> Arc<ToleMeta> {
    ToleMeta::new(alloc_id(), owner)
}
