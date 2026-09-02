// StackWindow — 任务栈 slot（窗口适配层：user 段上的栈领域策略）。
//
// slot = 守护页 + 栈体，一次从 user 段取段（lowest first-fit 自低端起排槽）：
// 守护页 `reserve(Guard)`（溢出缺页可诊断）、栈体 `claim`（立即物化——内核
// 任务栈在 S 态运行，若懒分配缺页会走内核地址 fatal 路径，故必须预分配帧）。
// 产物是 [`Span`]（**覆盖整个 slot**：va = slot 基址、size = guard + 栈体），
// 经 [`Space::release`] 按 Task 退役一次精确归还。

use crate::layout::TASK_STACK_GUARD;
use crate::memory::manager::MapError;
use crate::memory::manager::entry::PteFlags;

use super::super::core::Span;
use super::super::map::Pending;
use super::super::{Seg, Space};

/// 栈窗口（零状态策略）。
pub(crate) struct StackWindow;

impl StackWindow {
    /// 领一个任务栈 slot，返回 **slot 全区间** Span（含守护页）。
    ///
    /// `size` = 栈体大小（页对齐；guard 由本方法附加）。`kernel` = 所属空间
    /// 种类：用户空间栈需 U（用户 push）；内核空间栈不得带 U——S 态 SUM=0 下
    /// 访问 U 页会页故障，而内核任务跑 S 态。
    ///
    /// 栈体立即物化（`claim`，Eager）：逐页分配物理帧 + 装 PTE + 注入。守护页
    /// 只占簿记（`reserve(Guard)`，不物化）。
    ///
    /// 返回 Span：`va` = slot 基址（守护页底）、`size` = guard + 栈体总长、
    /// `pa` = None。栈体基址（供调用方算 stack_top）= `va + TASK_STACK_GUARD`。
    ///
    /// # Errors
    ///
    /// 段未就绪 / 段耗尽 / 物理帧耗尽 → [`MapError`]（回滚：已分配帧与段归还）。
    pub(crate) fn claim(space: &Space, size: usize, kernel: bool) -> Result<Span, MapError> {
        let slot_size = size + TASK_STACK_GUARD;
        space.with_flush(|inner| {
            let slot_va = inner.allocate(Seg::User, slot_size)?;
            // 守护页 Guard → 溢出缺页可诊断（只登记，不物化）
            let guard_flags = PteFlags::V | PteFlags::R | PteFlags::W;
            if let Err(e) =
                inner.reserve_map(slot_va, TASK_STACK_GUARD, guard_flags, Some(Pending::Guard))
            {
                // 装配失败：段退回（reserve 未落任何 PTE/帧）
                inner.deallocate(Seg::User, slot_va.as_usize(), slot_size);
                return Err(e);
            }
            // 栈体：立即物化（Eager）。逐页分配帧 + 装 PTE + 注入。
            let body_flags = if kernel {
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D
            } else {
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D
            };
            let body_va = slot_va + TASK_STACK_GUARD;
            if let Err(e) = inner.claim_map(body_va, size, body_flags) {
                // claim 已自回滚装配（清已装叶 + 摘 body map）；guard map 与段
                // 需整体退回——直接 remove slot 区间（清 guard 叶 + 摘 guard map）
                inner.unmap(slot_va, slot_size);
                inner.deallocate(Seg::User, slot_va.as_usize(), slot_size);
                return Err(e);
            }
            Ok(Span::new(Seg::User, slot_va, slot_size, None))
        })
    }
}
