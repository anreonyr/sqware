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
    /// **轮转游标**：`ready()` 下一次**从第几格起扫**。
    ///
    /// **照实记（这一格是量出来的）**：`ready()` 原先每轮都从第 0 格扫起、取第一枚就绪的——
    /// 于是装配期"最后挂上组"的那一位（`tid=20`）**一直排在后面**：它的手在孔上活了
    /// 1141~1228 ms（11 跑 5 跑），而那一秒里树**没有一趟**超过 200 ms、它那一格自
    /// `operator: arm late` 起就一直"是成员、有手"。⇒ 病根是**先看谁**，不是"谁算有事"。
    /// 今天命中一格之后把游标推到**它的下一格**：一格一格地公平。
    ///
    /// **只影响次序**：`await` 的契约仍是"等到**任意**一格有事"；游标只决定**先看谁**。
    /// `Relaxed` 就够（它是公平用的偏好，不是同步点；共享组上两个取用者抢它也不会错）。
    cursor: AtomicUsize,
}

pub(crate) fn key(meta: &ToleMeta) -> WakeKey {
    WakeKey::Tole { id: meta.id().0 }
}

/// **`cells()` 备不下**的次数（>0 = 有一次"组里明明有成员、却被报成空表"）。
static CELLS_SHORT: AtomicUsize = AtomicUsize::new(0);
/// **成员认不出对应 Pie**（`ready()` 里那一格 `continue`）的次数。**从前的静默格**。
pub(crate) static MATE_SKIP: AtomicUsize = AtomicUsize::new(0);

impl ToleMeta {
    fn new(id: ToleId, owner: TaskId) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, ToleState::Live),
            id,
            life: Life::new(),
            cells: SpinLock::new_level(Level::L3, Vec::new()),
            owner,
            cursor: AtomicUsize::new(0),
        })
    }

    /// 轮转起点（`ready()` 从这一格起扫）。
    pub(crate) fn cursor(&self) -> usize {
        self.cursor.load(Ordering::Relaxed)
    }

    /// 记下"下一轮从这一格起扫"（命中项的下一格）。
    pub(crate) fn seek_cursor(&self, at: usize) {
        self.cursor.store(at, Ordering::Relaxed);
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
            let n = cells.len();
            drop(cells);
            // **照实记（这一格从前是静默的）**：备不下就返回**空表**，而 [`ready`] 拿到空表就答
            // "没有一格就绪" ⇒ **组里明明有就绪的成员、读的人却被支去睡**（症状：debug 档量到的
            // `operator: await miss waiting=1`——`peek` 说手在孔上、组说没事）。这一格要看得见：
            // 第一次当场报一行（`n` = 那一刻组里有几格）。
            if n > 0 && CELLS_SHORT.fetch_add(1, Ordering::Relaxed) == 0 {
                crate::putln!("tole: cells short n={}", n);
            }
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
