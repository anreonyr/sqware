// Hole — 数据过内核的管道。
//
// HoleMeta 是内核侧"门洞"：单槽消息缓冲 + 状态。用户态 Pie<HoleMeta>（含
// Weak<HoleMeta>）只持门闩，不参与数据。
//
// 数据面原语：`push` / `pull` / `seal`。创建：`unseal()` —— 建 Meta + 注册 memo +
// 全权 pie 落 self（返 token）。
//
// 阻塞语义不在 HoleMeta 内（v1 简化）：push/pull 槽满/槽空 → 立即返 Busy，调用方
// 经调度域 wait/wake 自旋。

use alloc::sync::Arc;

use crate::lock::{Level, SpinLock};

use super::memo::{self, Meta, ResourceId};
use super::pie::{AnyPie, HOLE_MSG_LEN, MailError, Permission};

/// Hole 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleState {
    Live,
    Dead,
}

/// Hole 数据面实体（Arc 持有；最后强引用 drop 时 Meta 释放）。
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

// ── 数据面原语 ──

/// 写消息入槽（需 rights & W）。
/// `Denied` = rights 不够；`Dead` = 已 seal；`Busy` = 槽满。
pub(crate) fn push(meta: &HoleMeta, msg: &[u8; HOLE_MSG_LEN]) -> Result<(), MailError> {
    if !meta.alive() {
        return Err(MailError::Dead);
    }
    let mut slot = meta.slot.lock();
    if slot.is_some() {
        return Err(MailError::Busy);
    }
    *slot = Some(*msg);
    Ok(())
}

/// 取消息出槽（需 rights & R）。
/// `Denied` = rights 不够；`Dead` = 已 seal；`Busy` = 槽空。
pub(crate) fn pull(meta: &HoleMeta) -> Result<[u8; HOLE_MSG_LEN], MailError> {
    if !meta.alive() {
        return Err(MailError::Dead);
    }
    let mut slot = meta.slot.lock();
    match slot.take() {
        Some(msg) => Ok(msg),
        None => Err(MailError::Busy),
    }
}

/// 封印 Hole（state = Dead + memo 移除）。
pub(crate) fn seal(meta: &HoleMeta, id: ResourceId) {
    *meta.state.lock() = HoleState::Dead;
    memo::remove(id);
}

// ── 创建 ──

use crate::work::room::scheduler::core::current;

/// 解封 Hole：建 Meta + 注册 memo + 全权 pie 落 self（vestor=None）。返 token。
pub(crate) fn unseal() -> Result<u64, MailError> {
    let arc = HoleMeta::new();
    let id = memo::alloc_id();
    memo::insert(id, Meta::Hole(arc.clone()));

    let pie = super::pie::new_pie(
        id,
        Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
        None, // 原始自持：无 vestor
        Arc::downgrade(&arc),
    );
    let token = pie.token();

    let task = current().running_task().ok_or(MailError::Denied)?;
    let mut pies = task.pies.lock();
    pies.push(AnyPie::Hole(pie));
    Ok(token)
}
