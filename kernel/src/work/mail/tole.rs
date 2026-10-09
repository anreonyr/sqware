use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::lock::{Level, SpinLock};

use env::{MailCondition, TaskId};

use crate::work::mail::hole::HoleId;
use crate::work::mail::nole::NoleId;
use crate::work::mail::pole::PoleId;
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
    Hole(HoleId, MailCondition),
    Nole(NoleId),
    /// **页上那一位**（架把铃并进页 ⇒ 页也能进组）。只有 `Pull` 一条方向，与门铃同。
    Pole(PoleId),
}

impl Mate {
    pub(crate) fn key(self) -> WakeKey {
        match self {
            Mate::Hole(id, dir) => WakeKey::Hole { hole: id.0, dir },
            Mate::Nole(id) => WakeKey::Nole { id: id.0 },
            Mate::Pole(id) => WakeKey::Pole { id: id.0 },
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

/// **一条状态订阅的描述**（订阅与取消共用同一个值；它不是 token，不发给调用方）。
///
/// [`Sub::key`] 是**派生量、不是字段**：两格各自映射到一个 `WakeKey`，于是"描述"与
/// "边"不可能不一致。两格都产不出 `Tole { .. }`——订阅的转发图因此**不可能成环**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sub {
    /// 观察某个任务的**退出收尾完成**（搭在既有的 `WakeKey::Task` 上）。
    TaskCompleted(TaskId),
    /// 观察**订阅者自己**能力的可观察状态改变。
    Capabilities(TaskId),
}

impl Sub {
    pub(crate) fn key(self) -> WakeKey {
        match self {
            Sub::TaskCompleted(id) => WakeKey::Task { id },
            Sub::Capabilities(id) => WakeKey::Capabilities { task: id },
        }
    }
}

pub struct ToleMeta {
    state: SpinLock<ToleState>,
    id: ToleId,
    life: Arc<Life>,
    cells: SpinLock<Vec<Cell>>,
    /// **第二张表**：状态订阅（与 `cells` 并列）。
    ///
    /// 不能并进 `cells`：那里的 [`Cell::live`] 过滤恰好会把"目标已销毁、完成提示还没被
    /// 消费"那一条滤掉——而那一格正是订阅要保的东西。
    subs: SpinLock<Vec<Sub>>,
    owner: TaskId,
    /// **轮转游标**：`ready()` 下一次**从第几格起扫**。
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
/// **成员认不出对应 Pie**（`ready()` 里那一格被吞掉）的次数。
pub(crate) static MATE_SKIP: AtomicUsize = AtomicUsize::new(0);

impl ToleMeta {
    fn new(id: ToleId, owner: TaskId) -> Arc<Self> {
        Self::try_new(id, owner).expect("tole allocation failed")
    }

    fn try_new(id: ToleId, owner: TaskId) -> Result<Arc<Self>, crate::memory::manager::MapError> {
        let life = Life::try_new()?;
        Arc::try_new(Self {
            state: SpinLock::new_level(Level::L3, ToleState::Live),
            id,
            life,
            cells: SpinLock::new_level(Level::L3, Vec::new()),
            subs: SpinLock::new_level(Level::L3, Vec::new()),
            owner,
            cursor: AtomicUsize::new(0),
        })
        .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
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
            if n > 0 && CELLS_SHORT.fetch_add(1, Ordering::Relaxed) == 0 {
                crate::putln!("tole: cells short n={}", n);
            }
            return Vec::new();
        }
        out.extend(cells.iter().filter(|c| c.live()).cloned());
        out
    }

    /// 这一组装没装状态订阅（`Accord` 的拒授判据用它：含订阅的组不许转授）。
    pub(crate) fn has_subs(&self) -> bool {
        !self.subs.lock().is_empty()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn subs_len(&self) -> usize {
        self.subs.lock().len()
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

/// 把一个**状态来源**登记进组：同描述重复登记幂等（不重装转发边、不重复留提示）。
///
/// `life` 是**来源那一侧**的弱寿命（`TaskCompleted` 用目标任务的，`CapabilitiesChanged`
/// 用订阅者自己的）——它就是这条转发边的存活凭据：来源已死则 `forward` 收尾那一趟
/// `prune` 会把站点连同这条边一起收掉。
///
/// 登记成功**先留一次待复核提示**：已有变化、已完成的任务以及登记那一刻的状态，
/// 都不依赖"未来再来一个事件"。
pub(crate) fn subscribe(meta: &ToleMeta, sub: Sub, life: Weak<Life>) -> Result<(), ToleFail> {
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    {
        let mut subs = meta.subs.lock();
        if subs.contains(&sub) {
            return Ok(());
        }
        if subs.try_reserve(1).is_err() {
            return Err(ToleFail::OoM);
        }
        subs.push(sub);
    }
    if messenger::forward(sub.key(), life, meta.id.0, meta.life()).is_err() {
        let mut subs = meta.subs.lock();
        if let Some(at) = subs.iter().position(|s| *s == sub) {
            subs.swap_remove(at);
        }
        return Err(ToleFail::OoM);
    }
    let _ = messenger::knock(key(meta), &meta.life());
    Ok(())
}

/// 按**已安装的订阅描述**取消；同描述重复取消无事。
///
/// 只认 `(source, target)` 这条描述：**不要求目标还在世**、不要求还能升级出它的域、
/// 也不要求它还在 heir 里。
pub(crate) fn unsubscribe(meta: &ToleMeta, sub: Sub) -> Result<(), ToleFail> {
    if !meta.alive() {
        return Err(ToleFail::Dead);
    }
    {
        let mut subs = meta.subs.lock();
        if let Some(at) = subs.iter().position(|s| *s == sub) {
            subs.swap_remove(at);
        }
    }
    messenger::unforward(sub.key(), meta.id.0);
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
    let subs = core::mem::take(&mut *meta.subs.lock());
    for s in &subs {
        messenger::unforward(s.key(), meta.id.0);
    }
    messenger::wipe(key(meta));
}

impl Drop for ToleMeta {
    fn drop(&mut self) {
        let cells = core::mem::take(&mut *self.cells.lock());
        for c in &cells {
            messenger::unforward(c.mate.key(), self.id.0);
        }
        let subs = core::mem::take(&mut *self.subs.lock());
        for s in &subs {
            messenger::unforward(s.key(), self.id.0);
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

pub(crate) fn try_meta(owner: TaskId) -> Result<Arc<ToleMeta>, crate::memory::manager::MapError> {
    ToleMeta::try_new(alloc_id(), owner)
}
