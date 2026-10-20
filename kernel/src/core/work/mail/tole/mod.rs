mod await_wait;
mod cell;
pub(crate) mod codec;
mod watch;

use super::Liveness;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::{life::Life, task::Task};
use crate::machine::sync::{Level, SpinLock};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use env::{TaskId, ToleAwaitReply, ToleEventSource, ToleFail};

pub(crate) use await_wait::AwaitWait;
pub(crate) use cell::Cell;
pub(crate) use watch::Watch;

const MAX: usize = 1024;

pub struct ToleMeta {
    state: SpinLock<Liveness>,
    pub(crate) id: usize,
    life: Arc<Life>,
    cells: SpinLock<Vec<Cell>>,
    owner: TaskId,
    cursor: AtomicUsize,
}
fn key(meta: &ToleMeta) -> WakeKey {
    WakeKey::Tole { id: meta.id }
}
impl ToleMeta {
    #[cfg(debug_assertions)]
    pub(crate) fn new(owner: TaskId) -> Arc<Self> {
        Self::try_new(owner).expect("tole allocation failed")
    }

    pub(crate) fn try_new(owner: TaskId) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        Arc::try_new(Self {
            state: SpinLock::new_level(Level::L3, Liveness::Live),
            id: super::next_id(),
            life: Life::try_new()?,
            cells: SpinLock::new_level(Level::L3, Vec::new()),
            owner,
            cursor: AtomicUsize::new(0),
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
        *self.state.lock() == Liveness::Live
    }
    pub(crate) fn has_cells(&self) -> bool {
        !self.cells.lock().is_empty()
    }
    #[cfg(debug_assertions)]
    pub(crate) fn cells_len(&self) -> usize {
        self.cells.lock().len()
    }
    pub(crate) fn poll(&self, task: &Arc<Task>) -> Result<ToleAwaitReply, ToleFail> {
        if !self.alive() {
            return Err(ToleFail::Dead);
        }
        let count = self.cells.lock().len();
        if count == 0 {
            return Ok(ToleAwaitReply::Pending);
        }
        let start = self.cursor.load(Ordering::Relaxed) % count;
        let mut own = false;
        for offset in 0..count {
            let at = (start + offset) % count;
            let cell = { self.cells.lock().get(at).cloned() };
            let Some(cell) = cell else { continue };
            if cell.actor != task.ident.id {
                continue;
            }
            own = true;
            let status = cell.poll(task);
            if let Some(fail) = status {
                self.cursor.store((at + 1) % count, Ordering::Relaxed);
                return Ok(ToleAwaitReply::Source {
                    source: cell.watch.source(),
                    fail,
                });
            }
        }
        if own {
            Ok(ToleAwaitReply::Pending)
        } else {
            Err(ToleFail::Denied)
        }
    }
}
pub(crate) fn attach(meta: &ToleMeta, cell: Cell) -> Result<(), ToleFail> {
    let _commit = crate::core::work::commit();
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    let mut shared = [false; 4];
    {
        let mut cells = meta.cells.lock();
        if cells
            .iter()
            .any(|old| old.watches(cell.actor, cell.watch.source()))
        {
            return Ok(());
        }
        if cells.len() >= MAX {
            return Err(ToleFail::OoM);
        }
        cells.try_reserve(1).map_err(|_| ToleFail::OoM)?;
        for (at, (key, _)) in cell.keys.iter().enumerate() {
            shared[at] = cells.iter().any(|old| old.covers(*key));
        }
    }
    for (at, (key, life)) in cell.keys.iter().enumerate() {
        if !shared[at] && messenger::forward(*key, life.clone(), meta.id, meta.life()).is_err() {
            for (index, (key, _)) in cell.keys.iter().take(at).enumerate() {
                if !shared[index] {
                    messenger::unforward(*key, meta.id);
                }
            }
            return Err(ToleFail::OoM);
        }
    }
    meta.cells.lock().push(cell);
    messenger::knock(key(meta), &meta.life());
    Ok(())
}
pub(crate) fn detach(
    meta: &ToleMeta,
    actor: TaskId,
    source: ToleEventSource,
) -> Result<(), ToleFail> {
    detach_where(meta, actor, |known| known == source)
}

fn detach_where(
    meta: &ToleMeta,
    actor: TaskId,
    matches: impl Fn(ToleEventSource) -> bool,
) -> Result<(), ToleFail> {
    let _commit = crate::core::work::commit();
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    let cell = {
        let mut cells = meta.cells.lock();
        cells
            .iter()
            .position(|c| c.actor == actor && matches(c.watch.source()))
            .map(|at| cells.remove(at))
    };
    if let Some(cell) = cell {
        for (key, _) in cell.keys.iter() {
            let shared = meta.cells.lock().iter().any(|c| c.covers(*key));
            if !shared {
                messenger::unforward(*key, meta.id);
            }
        }
    }
    messenger::knock(key(meta), &meta.life());
    Ok(())
}
pub(crate) fn seal(meta: &ToleMeta) {
    let _commit = crate::core::work::commit();
    clear(meta);
}
fn clear(meta: &ToleMeta) {
    *meta.state.lock() = Liveness::Dead;
    messenger::wipe(WakeKey::seal(env::PieKind::Tole, meta.id));
    let cells = core::mem::take(&mut *meta.cells.lock());
    for cell in cells {
        for (key, _) in cell.keys.iter() {
            messenger::unforward(*key, meta.id);
        }
    }
    messenger::wipe(key(meta));
}
impl Drop for ToleMeta {
    fn drop(&mut self) {
        clear(self);
    }
}

unsafe impl Send for ToleMeta {}
unsafe impl Sync for ToleMeta {}
