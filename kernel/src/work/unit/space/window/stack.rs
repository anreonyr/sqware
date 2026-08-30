// StackWindow — 任务栈 slot（窗口适配层：user 段上的栈领域策略）。
//
// slot = 守护页 + 栈体，一次从 user 段取段（lowest first-fit 自低端起排槽）：
// 守护页登记 `Pending::Guard`（溢出缺页可诊断）、栈体**立即物化**（Eager——
// 内核任务栈在 S 态运行，若懒分配缺页会走内核地址 fatal 路径，故必须预分配帧）。
// 产物是 [`Span`]（**覆盖整个 slot**：va = slot 基址、size = guard + 栈体），
// 经 [`Space::release`] 按 Task 退役一次精确归还。

use alloc::boxed::Box;

use crate::layout::TASK_STACK_GUARD;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::Frame;

use super::super::{Seg, Space};
use super::super::core::Span;
use super::super::map::{Map, Pending};

/// 栈窗口（零状态策略）。
pub(crate) struct StackWindow;

impl StackWindow {
    /// 领一个任务栈 slot，返回 **slot 全区间** Span（含守护页）。
    ///
    /// `size` = 栈体大小（页对齐；guard 由本方法附加）。`kernel` = 所属空间
    /// 种类：用户空间栈需 U（用户 push）；内核空间栈不得带 U——S 态 SUM=0 下
    /// 访问 U 页会页故障，而内核任务跑 S 态。
    ///
    /// 栈体立即物化（Eager）：逐页分配物理帧 + 装 PTE + 注入 map。守护页只
    /// 占簿记（Guard，不物化）。
    ///
    /// 返回 Span：`va` = slot 基址（守护页底）、`size` = guard + 栈体总长、
    /// `pa` = None。栈体基址（供调用方算 stack_top）= `va + TASK_STACK_GUARD`。
    ///
    /// # Errors
    ///
    /// 段未就绪 / 段耗尽 / 物理帧耗尽 → [`MapError`]（回滚：已分配帧与段归还）。
    pub(crate) fn claim(
        space: &Space,
        size: usize,
        kernel: bool,
    ) -> Result<Span, MapError> {
        let slot_size = size + TASK_STACK_GUARD;
        space.with_flush(|inner| {
            let user = inner.user.as_mut().ok_or(MapError::NoRegion)?;
            let slot_base = user.allocate(slot_size).map_err(|_| MapError::OutOfMemory)?;
            let slot_va = VirtAddr::from_raw(slot_base);
            // 守护页 Guard → 溢出缺页可诊断
            inner.maps.push(Map::new(
                slot_va,
                TASK_STACK_GUARD,
                PteFlags::V | PteFlags::R | PteFlags::W,
                Some(Pending::Guard),
                alloc::vec::Vec::new(),
            ));
            // 栈体：立即物化（Eager）。逐页分配帧 + 装 PTE + 注入。
            let body_flags = if kernel {
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D
            } else {
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D
            };
            let body_va = slot_va + TASK_STACK_GUARD;
            inner.maps.push(Map::new(
                body_va,
                size,
                body_flags,
                None, // Eager（全物化）
                alloc::vec::Vec::new(),
            ));
            let pages = size / PAGE_SIZE;
            for i in 0..pages {
                let page: Frame = unsafe {
                    Box::try_new_zeroed_in(allocator())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                };
                let pa = PhysAddr::from_raw(page.as_ptr() as usize);
                let m_va = body_va + i * PAGE_SIZE;
                if inner.root.map(m_va, pa, PAGE_SIZE, body_flags).is_err() {
                    // 回滚：清已映射叶 + 摘 map + 段归还
                    for j in 0..i {
                        inner.root.unmap(body_va + j * PAGE_SIZE);
                    }
                    inner.user.as_mut().expect("user exists").deallocate(slot_base, slot_size);
                    return Err(MapError::OutOfMemory);
                }
                inner
                    .maps
                    .iter_mut()
                    .find(|m| m.va == body_va)
                    .expect("body map exists")
                    .inject(page);
            }
            Ok(Span::new(Seg::User, slot_va, slot_size, None))
        })
    }
}
