// Tole — 若干枚孔挂在一处，等其中任意一格有事。
//
// # 名字
//
// `Tole`（收费站）与 `Hole` / `Pole` / `Nole` 同族，四个名字各说一件事：
//
//     Hole  有槽（消息穿孔）       —— 有数据面
//     Pole  有页（页视图）         —— 有数据面
//     Nole  **什么都没有**         —— 无数据面
//     Tole  自己不装东西，**记着别人** —— 无自带数据面，只有一张格子表
//
// # 它是什么
//
// 一枚孔只能等一个方向、一枚线程只能等一个键。`Tole` 是"**多路等待**"的资源面：
// 一张格子表，每格 = 一枚目标孔的**身份 + 存活单元** + 一个方向。等待侧拿这张表
// 去登记，任意一格有事即醒来（登记在 `room` 那一侧，不属本模块）。
//
// **格子不持强引用**：目标孔最后一份门闩消失时格子自然失效（`Life` 判死）——格子
// 不延长任何资源的寿命，也不新增"已死"这种状态。
//
// 数据面原语（全部非阻塞）：
// - `meta(owner)`：造一个空架子。
// - `hang(meta, hole, dir)`：挂上一格；同（孔，方向）幂等。同时把本组登记成那一格的
//   **转发目标**（那一枚孔的站点在投信时要顺带叫醒本组），并叫醒等本组的人重取快照。
// - `unhang(meta, hole, dir)`：摘下一格（连转发登记一起摘）；没挂过即无事。
// - `seal(meta)` / `Drop`：置死、清表、**撤掉全部转发登记**，并 `wipe` 本组的键
//   ——成员键退役（`wipe`）时同样会叫醒等本组的人。
// - `wait_tole(meta, dur)`：等到任意一格有事。
//
// **锁序约定**：`cells` 是 L3 锁。本模块**不在持 `cells` 时**调 `messenger`
// （站点表同为 L3，3→3 禁止）：先改表、放开锁，再登记转发／叫醒。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::lock::{Level, SpinLock};

use env::HoleDir;

use crate::work::mail::hole::{self, HoleId, HoleMeta};
use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::gate::GateError;
use core::time::Duration;
use crate::work::unit::life::Life;

/// Tole 的全局身份（自 1 递增、永不复用）。
///
/// 与 `HoleId` / `NoleId` **各起一份计数器**：身份只在各自的命名空间里比，共用一份
/// 只会把两个模块的寿命绑在一起。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToleId(pub usize);

fn alloc_id() -> ToleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    ToleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// Tole 状态。与 Hole / Pole / Nole 同词：`Seal` 之后恒 `Dead`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToleState {
    Live,
    Dead,
}

/// 一格：**哪一枚孔的哪一个方向**。
///
/// 记 `HoleId` 而不记 `PieToken`：句柄是**表**的身份（只在持有它的那张表里有意义），
/// 而架子是资源、要被别的表使用——故格子记的是**资源**的身份（内核命名、永不复用）。
///
/// `life` 是目标孔的存活单元（弱引用）：孔没了，这一格自然失效——不需要摘。
#[derive(Clone)]
pub(crate) struct Cell {
    hole: HoleId,
    dir: HoleDir,
    life: Weak<Life>,
}

impl Cell {
    /// 这一格还指得着东西吗（目标孔仍活着）。
    pub(crate) fn live(&self) -> bool {
        !Life::dead(&self.life)
    }

    /// 目标孔的**资源身份**（号是表里的东西，故这里只给身份）。
    pub(crate) fn hole(&self) -> HoleId {
        self.hole
    }

    /// 这一格关心的是哪个方向。
    pub(crate) fn dir(&self) -> HoleDir {
        self.dir
    }
}

/// Tole 数据面实体（Arc 持有；**无自带载荷**，只有格子表）。
pub struct ToleMeta {
    state: SpinLock<ToleState>,
    /// 本 Tole 的全局资源 id（单调分配、永不复用）。
    id: ToleId,
    /// 本 Tole 的存活单元：强持有者是本 Meta ⇒ 最后一份门闩消失时它自然判死。
    life: Arc<Life>,
    /// 格子表。**不持任何强引用**（见 [`Cell`]）；摘除只在显式 `unhang` 与封印时发生。
    cells: SpinLock<Vec<Cell>>,
    /// 开辟者：`UnsealTole` 时的任务 id（构造期定型，无 setter）。0 = 内核自建。
    /// 语义同 `HoleMeta::owner`：`vestor` 管门闩的来历，`owner` 管资源的来历。
    owner: usize,
}

/// 本组的等待键：组自己的身份（`WakeKey::Tole`，与 `Hole{id}`/`Nole{id}` 同构）。
pub(crate) fn key(meta: &ToleMeta) -> WakeKey {
    WakeKey::Tole { id: meta.id().0 }
}

impl ToleMeta {
    fn new(id: ToleId, owner: usize) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, ToleState::Live),
            id,
            life: Life::new(),
            // 空表：零分配（第一条 `hang` 才要容量）。
            cells: SpinLock::new_level(Level::L3, Vec::new()),
            owner,
        })
    }

    /// 本 Tole 的存活单元（弱引用）。与 `HoleMeta::life` 同款：`Weak` 就在 Meta 里，
    /// 取值是纯函数。
    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    /// 本 Tole 的全局身份。
    pub(crate) fn id(&self) -> ToleId {
        self.id
    }

    /// 资源开辟者（见字段 `owner`）。
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    /// 资源可用（已封印 → false）。
    pub(crate) fn alive(&self) -> bool {
        matches!(*self.state.lock(), ToleState::Live)
    }

    /// 当前**还有效**的格子（目标孔已死的格子不出现）。
    ///
    /// 交给等待侧登记用：拿到的是快照，登记期间格子可能失效——失效只会让那一格
    /// 永远等不到，不会叫错人（等待侧按身份复核）。
    pub(crate) fn cells(&self) -> Vec<Cell> {
        let cells = self.cells.lock();
        let mut out = Vec::new();
        if out.try_reserve(cells.len()).is_err() {
            // 备不出容量 ⇒ 当作"这一刻没有格子"：等待侧下一轮再来。
            return Vec::new();
        }
        out.extend(cells.iter().filter(|c| c.live()).cloned());
        out
    }
}

// ── 数据面原语（非阻塞）──

/// 挂上一格：**同（孔，方向）幂等**（重复挂不叠加）。
///
/// 前置：`hole` 是调用方表里的那一枚（判权在 envcall 入口，数据面不感知 rights）。
///
/// 目标孔已死也允许挂（那一格当场就是失效的）——"挂上"这件事与"还活着"无关，
/// 少一种要调用方分辨的状态。
pub(crate) fn hang(meta: &ToleMeta, hole: &HoleMeta, dir: HoleDir) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let cell = Cell {
        hole: hole.id(),
        dir,
        life: hole.life(),
    };
    let want = (cell.hole, cell.dir);
    {
        let mut cells = meta.cells.lock();
        if cells
            .iter()
            .any(|c| c.hole == cell.hole && c.dir == cell.dir)
        {
            return Ok(());
        }
        if cells.try_reserve(1).is_err() {
            return Err(GateError::OoM);
        }
        cells.push(cell);
    }
    // **锁外**登记转发（站点表是 L3，与 `cells` 不嵌套）：那一枚孔今后一投信，
    // 也认醒本组。登记不上（站点表/转发格满）⇒ 把刚挂的那一格退回，不留下
    // "挂着却叫不醒"的半截状态。
    if messenger::forward(hole::key(hole, dir), hole.life(), meta.id.0).is_err() {
        let mut cells = meta.cells.lock();
        if let Some(at) = cells.iter().position(|c| (c.hole, c.dir) == want) {
            cells.swap_remove(at);
        }
        return Err(GateError::OoM);
    }
    // 叫醒等本组的人：快照变了，它们该重取一遍（信标只是提示，醒来自己复核）。
    let _ = messenger::wake(key(meta), &meta.life());
    Ok(())
}

/// 摘下一格：**没挂过即无事**（不返错）。
///
/// 只摘这一格；同孔的另一个方向若也挂着，照旧留着。
pub(crate) fn unhang(meta: &ToleMeta, hole: &HoleMeta, dir: HoleDir) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let id = hole.id();
    {
        let mut cells = meta.cells.lock();
        if let Some(at) = cells.iter().position(|c| c.hole == id && c.dir == dir) {
            cells.swap_remove(at);
        }
    }
    // 锁外撤转发登记（同 `hang` 的锁序）。没挂过也照撤：幂等，且不留下"
    // 格已摘、投信还叫本组"的多余一跳。
    messenger::unforward(hole::key(hole, dir), meta.id.0);
    Ok(())
}

// ── 封印 ──

/// 封印 tole：清空格子表并置死。
///
/// **不回收内存**——资源寿命由引用计数决定：最后一份门闩消失时 Meta 释放。
/// 调用方不持 L3 锁。
pub(crate) fn seal(meta: &ToleMeta) {
    *meta.state.lock() = ToleState::Dead;
    retire(meta);
}

// ── 收尾 ──

/// 清空表格 + 撤掉全部转发登记 + `wipe` 本组的键（放行等本组的人）。
///
/// 三件事都必须做：格子走了，成员孔那一侧的"投信还叫本组"就成了多余的一跳；
/// 而等本组的人若不叫醒，就会抱着一个空快照睡到期限（有界等待会退化成"每次都等满"）。
///
/// 调用方义务：**在锁外**调（站点表是 L3，且 `wipe` 会 `rise` 任务）。
fn retire(meta: &ToleMeta) {
    let cells = core::mem::take(&mut *meta.cells.lock());
    for c in &cells {
        messenger::unforward(WakeKey::Hole { hole: c.hole.0, dir: c.dir }, meta.id.0);
    }
    messenger::wipe(key(meta));
}

impl Drop for ToleMeta {
    /// 最后一份强引用消失：与 [`hole::HoleMeta`] 的 `Drop` 同形——成员孔那一侧不再
    /// 记得本组，等本组的人当场放行（键随即判死，站点随 `prune` 走）。
    fn drop(&mut self) {
        let cells = core::mem::take(&mut *self.cells.lock());
        for c in &cells {
            messenger::unforward(WakeKey::Hole { hole: c.hole.0, dir: c.dir }, self.id.0);
        }
        messenger::wipe(key(self));
    }
}

// ── 等待（唯一入口）──

/// 等到**任意一格**有事：`Handoff` 的含义与 `fall`／`join` 同款——挂起过一侧恢复后
/// 读到的恒是预置值，故调用方须按 deadline 循环，醒来自己按 [`ToleMeta::cells`] 的
/// 快照复核（信标只是提示）。
///
/// `dur == ZERO` 走到 `block` 的"只探测"那一支：站点在而链空 ⇒ 置信标、当场返回。
pub(crate) fn wait_tole(meta: &ToleMeta, dur: Duration) -> Result<Handoff<()>, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    messenger::wait(key(meta), meta.life(), dur)
}

// ── 创建 ──

/// 造一个空架子的资源实体：建 Meta。**不落 pies**——建门闩与落 `task.pies` 由
/// envcall 编排（gate::new_pie + pies.push）。返 `Arc`：它既是资源实体，也是门闩
/// 持有的**唯一强引用**（资源寿命 = 能力寿命）。
///
/// `owner` = 开辟者任务 id（envcall 入口传当前任务）。
pub(crate) fn meta(owner: usize) -> Arc<ToleMeta> {
    ToleMeta::new(alloc_id(), owner)
}
