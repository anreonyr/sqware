// hole — 数据过内核的管道。
//
// holeMeta 是内核侧"门洞"：单槽消息缓冲 + 状态。用户态 Pie<HoleMeta>（含
// Weak<HoleMeta>）只持门闩，不参与数据。
//
// 数据面原语（全部非阻塞）：
// - `try_push` / `try_pull`：槽满/空返 Busy；成功即唤醒对侧。
// - `ready(dir)`：该方向现在可用吗。
// - `wait(meta, dir, dur)`：唯一挂起入口——先探、后挂；死则报 Dead。
// - `key(meta, dir)`：该方向的等待键（命名空间 0，键不出内核）。
//
// 写/读完槽后都 wake 对侧 waiters。

use alloc::sync::Arc;
use core::time::Duration;

use crate::lock::{Level, SpinLock};

use env::HoleDir;

use super::HOLE_MSG_LEN;
use super::memo::{self, Meta, ResourceId};
use crate::work::room::messenger::{self, Handoff, WaitKey};
use crate::work::unit::gate::GateError;

/// hole 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleState {
    Live,
    Dead,
}

/// hole 数据面实体（Arc 持有；最后强引用 drop 时 Meta 释放）。
pub struct HoleMeta {
    state: SpinLock<HoleState>,
    /// 单槽消息缓冲（Some = 消息在途）。
    slot: SpinLock<Option<[u8; HOLE_MSG_LEN]>>,
}

impl HoleMeta {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            slot: SpinLock::new_level(Level::L3, None),
        })
    }

    /// 存活：state == Live（Arc 仍有效由 Pie 持 Weak 保证）。
    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == HoleState::Live
    }

    /// 该方向现在可用吗：`Pull` = 槽里有消息，`Push` = 槽空。纯查询，不判存活。
    ///
    /// 与 `wait` 的挂起条件、与 `try_push`/`try_pull` 的唤醒点读同一份 `slot`——
    /// 三者必须同源，否则会出现"就绪了却没人唤醒"或"唤醒后仍不满足"。
    pub(crate) fn ready(&self, dir: HoleDir) -> bool {
        let slot = self.slot.lock();
        match dir {
            HoleDir::Pull => slot.is_some(),
            HoleDir::Push => slot.is_none(),
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
    }
}

// ── 等待 key（per-meta × 方向）──

/// 方向 → 等待键。命名空间恒 0（内核），故用户态构不出同键（键不出内核）；
/// 低位是方向位，两个方向的键必不相同。
///
/// 等数据的 task 等 `Pull` 键（push 写完槽后唤醒），等空位的 task 等 `Push` 键
/// （pull 取完槽后唤醒）。
pub(crate) fn key(meta: &HoleMeta, dir: HoleDir) -> WaitKey {
    let raw = meta as *const _ as usize;
    match dir {
        // 显式把 `raw | 1` 拆成两个语句、命名中间变量：size 优化下 `| 1` 不会再
        // 合并进 `WaitKey::compose` 的 mask 计算路径（见 §13.10 A 待办方向 B）。
        HoleDir::Pull => {
            let with_low_bit = raw | 1;
            WaitKey::compose(0, with_low_bit)
        }
        // 显式把 raw 拆出来给变量：避免 size 优化把 `| 1` 折叠到 compose 的 mask
        // 参数里（见 §13.10 A 待办方向 B）。
        HoleDir::Push => WaitKey::compose(0, raw),
    }
}

// ── 数据面原语（非阻塞）──

/// 非阻塞 push：槽空则写入并 wake 等读的；槽满返 Busy。
pub(crate) fn try_push(meta: &HoleMeta, msg: &[u8; HOLE_MSG_LEN]) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let mut slot = meta.slot.lock();
    if slot.is_some() {
        return Err(GateError::Busy);
    }
    *slot = Some(*msg);
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Pull));
    Ok(())
}

/// 非阻塞 pull：槽非空则取走并 wake 等写的；槽空返 Busy。
pub(crate) fn try_pull(meta: &HoleMeta) -> Result<[u8; HOLE_MSG_LEN], GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let mut slot = meta.slot.lock();
    match slot.take() {
        Some(msg) => {
            drop(slot);
            let _ = messenger::wake(key(meta, HoleDir::Push));
            Ok(msg)
        }
        None => Err(GateError::Busy),
    }
}

// ── 挂起（唯一入口）──

/// 等待结果（核心语义，不含 ABI 映射）。
pub(crate) enum Waited {
    /// 未挂起：`true` = 该方向现在就绪。
    Resume(bool),
    /// 已挂起：`Some(pa)` = 切到该帧；`None` = 本核无后备（调用方取活）。
    Parked(Option<usize>),
}

/// 等某方向就绪：死 → `Err(Dead)`；就绪或 `dur == 0` → 不挂起；否则挂起。
///
/// 前置：调用者在任务上下文，且**不持任何 L3 锁**（`wait_sites` 是 L3，3→3 禁止）。
///
/// 「先探」在此处不可省：对侧可能已经写入并正等我们取，此时若我们 park 在"等写入"
/// 上就永远等不到下一次唤醒。先探与登记之间的窗口由 messenger 的 pend 双检封住
/// （窗口内的 wake 置 pend，登记时被消费 ⇒ 不挂起）。
pub(crate) fn wait(meta: &HoleMeta, dir: HoleDir, dur: Duration) -> Result<Waited, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    if meta.ready(dir) {
        return Ok(Waited::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Waited::Resume(false));
    }
    Ok(match messenger::wait(key(meta, dir), dur) {
        // 窗口内 wake 已至（未挂起）：以当前状态为准。
        Handoff::Resume => Waited::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Waited::Parked(Some(pa)),
        Handoff::Idle => Waited::Parked(None),
    })
}

// ── 封印 ──

/// 封印 hole：置死 → 唤醒两个方向的**全部**等待者 → memo 移除。
///
/// 唤醒必须在 `memo::remove` **之前**：最后一份强引用就在 memo 表里，remove 在
/// L3 锁内 drop，此时再 wake（sites 亦 L3）构成 3→3 嵌套。`wake` 只弹队首，故
/// 循环到空；被唤醒者重判 alive 得 Dead，不会挂死。
pub(crate) fn seal(meta: &HoleMeta, id: ResourceId) {
    *meta.state.lock() = HoleState::Dead;
    while messenger::wake(key(meta, HoleDir::Pull)) {}
    while messenger::wake(key(meta, HoleDir::Push)) {}
    memo::remove(id);
}

// ── 创建 ──

/// 解封 hole 的资源实体：建 Meta + 注册 memo。**不落 pies**——建门闩与落
/// `task.pies` 由 envcall 编排（gate::new_pie + pies.push）。返 `(Arc, ResourceId)`：
/// `ResourceId` 供 gate::new_pie 第一参，`Arc` 供 Weak<HoleMeta>。
pub(crate) fn meta() -> Result<(Arc<HoleMeta>, ResourceId), GateError> {
    let arc = HoleMeta::new();
    let id = memo::alloc_id();
    memo::insert(id, Meta::Hole(arc.clone()));
    Ok((arc, id))
}
