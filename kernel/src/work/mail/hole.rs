// Hole — 数据过内核的管道。
//
// HoleMeta 是内核侧"门洞"：单槽消息缓冲 + 状态。
// 用户态 Pie<Hole>（含 Weak<HoleMeta>）只持门闩，不参与数据。
//
// 数据面原语：`hole_push` / `hole_pull` / `hole_shut`。
// 创建：`hole_create()` —— 建 Meta + 注册 + 推 AnyPie::Hole 到当前 Task.pies。
//
// 阻塞语义不在 HoleMeta 内（v1 简化）：push/pull 槽满/槽空 → 立即返 Busy，调用方
// 经调度域 wait/wake 自旋（与现行一致）。wait 键可基于 pie_idx（每个 Task 独立）。

use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use crate::lock::{Level, SpinLock};

use super::pie::{HOLE_MSG_LEN, MailError, R, W};
use super::resource_table::{self, ResourceId};

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
/// `Denied` = rights 不够；`Dead` = 已 shut；`Busy` = 槽满。
pub(crate) fn hole_push(meta: &HoleMeta, msg: &[u8; HOLE_MSG_LEN]) -> Result<(), MailError> {
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
/// `Denied` = rights 不够；`Dead` = 已 shut；`Busy` = 槽空。
pub(crate) fn hole_pull(meta: &HoleMeta) -> Result<[u8; HOLE_MSG_LEN], MailError> {
    if !meta.alive() {
        return Err(MailError::Dead);
    }
    let mut slot = meta.slot.lock();
    match slot.take() {
        Some(msg) => Ok(msg),
        None => Err(MailError::Busy),
    }
}

/// 终止 Hole（state = Dead + 资源表移除）。
pub(crate) fn hole_shut(meta: &HoleMeta, id: ResourceId) {
    *meta.state.lock() = HoleState::Dead;
    resource_table::remove(id, super::pie::PieKind::Hole);
}

// ── 创建 ──

use crate::work::room::scheduler::core::current;

/// 创建 Hole：建 Meta + 注册 + 推 AnyPie::Hole 到当前 Task.pies。
/// 返 pie_idx（Task 内 Vec 索引）。
pub(crate) fn hole_create() -> Result<usize, MailError> {
    let arc = HoleMeta::new();
    let id = resource_table::alloc_id();
    resource_table::insert_hole(id, &arc);

    let pie = super::pie::new_pie::<super::pie::Hole>(
        id,
        R | W,
        alloc::sync::Arc::downgrade(&arc),
    );

    let task = current()
        .running_task()
        .ok_or(MailError::Denied)?;
    let idx = super::pie::next_pie_idx();
    task.pies.lock().push(super::pie::AnyPie::Hole(pie));
    Ok(idx)
}

#[allow(dead_code)]
fn _ordering_anchor(_: Ordering) {}