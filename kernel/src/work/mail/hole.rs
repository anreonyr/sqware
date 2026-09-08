// hole — 数据过内核的管道。
//
// holeMeta 是内核侧"门洞"：单槽消息缓冲 + 状态。用户态 Pie<HoleMeta>（含
// Weak<HoleMeta>）只持门闩，不参与数据。
//
// 数据面原语：
// - `push` / `pull`：真阻塞。槽满/空时 park_mail（BlockReason::Mail），对方完成
//   对侧后 wake 解锁。用于 ktask 闭包（走 asm wait_mail）。
// - `try_push` / `try_pull`：非阻塞。槽满/空返 Busy。用于 envcall handler
//   （utask 不能 fire-and-forget park）。
// - `push_key` / `pull_key`：wait_mail 用的 wait key（per-meta）。
//
// 写/读完槽后都 wake 对侧 waiters。

use alloc::sync::Arc;

use crate::lock::{Level, SpinLock};

use super::memo::{self, Meta, ResourceId};
use super::HOLE_MSG_LEN;
use crate::work::room::messenger::{self, WaitKey};
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
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
    }
}

// ── 等待 key（push/pull 方向各一）──

/// 等空位的 task 用 push_key（push 写完槽后唤醒对方等读的）。
pub(crate) fn push_key(meta: &HoleMeta) -> WaitKey {
    let raw = meta as *const _ as usize;
    // 显式把 raw 拆出来给变量：避免 size 优化把 `| 1` 折叠到 compose 的 mask
    // 参数里（见 §13.10 A 待办方向 B）。
    WaitKey::compose(0, raw)
}

/// 等数据的 task 用 pull_key（pull 取完槽后唤醒对方等写的）。
pub(crate) fn pull_key(meta: &HoleMeta) -> WaitKey {
    let raw = meta as *const _ as usize;
    // 显式把 `raw | 1` 拆成两个语句、命名中间变量：size 优化下 `| 1` 不会再
    // 合并进 `WaitKey::compose` 的 mask 计算路径（见 §13.10 A 待办方向 B）。
    let with_low_bit = raw | 1;
    WaitKey::compose(0, with_low_bit)
}

// ── 数据面原语（真阻塞，ktask 用）──

/// 阻塞 push：槽空则写入并 wake pull waiters；槽满则 park_mail，等 pull 后唤醒重试。
pub(crate) fn push(meta: &HoleMeta, msg: &[u8; HOLE_MSG_LEN]) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    loop {
        {
            let mut slot = meta.slot.lock();
            if slot.is_none() {
                *slot = Some(*msg);
                drop(slot);
                let _ = messenger::wake(pull_key(meta));
                return Ok(());
            }
        }
        messenger::park_mail(push_key(meta));
        if !meta.alive() {
            return Err(GateError::Dead);
        }
    }
}

/// 阻塞 pull：槽非空则取走并 wake push waiters；槽空则 park_mail，等 push 后唤醒重试。
pub(crate) fn pull(meta: &HoleMeta) -> Result<[u8; HOLE_MSG_LEN], GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    loop {
        {
            let mut slot = meta.slot.lock();
            if let Some(msg) = slot.take() {
                drop(slot);
                let _ = messenger::wake(push_key(meta));
                return Ok(msg);
            }
        }
        messenger::park_mail(pull_key(meta));
        if !meta.alive() {
            return Err(GateError::Dead);
        }
    }
}

// ── 数据面原语（非阻塞，envcall handler 用）──

/// 非阻塞 push：槽空则写入并 wake；槽满返 Busy。
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
    let _ = messenger::wake(pull_key(meta));
    Ok(())
}

/// 非阻塞 pull：槽非空则取走并 wake；槽空返 Busy。
pub(crate) fn try_pull(meta: &HoleMeta) -> Result<[u8; HOLE_MSG_LEN], GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let mut slot = meta.slot.lock();
    match slot.take() {
        Some(msg) => {
            drop(slot);
            let _ = messenger::wake(push_key(meta));
            Ok(msg)
        }
        None => Err(GateError::Busy),
    }
}

// ── 封印 ──

/// 封印 hole（state = dead + memo 移除）。
pub(crate) fn seal(meta: &HoleMeta, id: ResourceId) {
    *meta.state.lock() = HoleState::Dead;
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
