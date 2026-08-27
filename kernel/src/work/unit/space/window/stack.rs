// StackWindow — 任务栈窗口（动态侧：栈 slot）。
//
// 顶锚于用户空间上界之下 `[upper() − STACK_WINDOW_SIZE, upper())`。slot = 守护页 +
// 栈体两个子 Map（owner = 线程 id）：守护页 Reserved（溢出缺页可诊断）、栈体
// Anonymous（帧随后经 `SpaceInner::attach_dynamic` 注入）——claim 只占 VA 簿记、
// 不物化帧。生命周期 owner 制：claim(owner) / reclaim(owner) 成对，线程退役回收。

use alloc::vec::Vec;

use crate::layout::{STACK_WINDOW_SIZE, TASK_STACK_GUARD};
use crate::memory::allocator::interval::Direction;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::{MapError, mode};

use super::super::dynamic::Dynamic;
use super::super::map::{Map, MapKind};

/// 栈窗口。
#[derive(Debug)]
pub(crate) struct StackWindow {
    /// 公共窗口核心（区间分配器 + 子 Map 表）。
    pub(crate) inner: Dynamic,
}

impl StackWindow {
    /// 构造：顶锚于用户空间上界之下 `[upper() − STACK_WINDOW_SIZE, upper())`。
    pub(crate) fn new() -> Self {
        let upper = mode::upper().as_usize();
        Self {
            inner: Dynamic::window(upper - STACK_WINDOW_SIZE, upper),
        }
    }

    /// 领一个任务栈 slot，返回栈体 VA（向下增长，底部守护页）。
    ///
    /// slot = 守护页 + 栈体，一次从窗口领取；守护页登记 Reserved、栈体登记
    /// Anonymous（帧随后经 attach_dynamic 注入）——claim 只占簿记、不物化帧。
    /// 两子 Map 均标 `owner`，退出时按 owner 一并回收（[`Self::reclaim`]）。
    ///
    /// `kernel` = 所属空间种类：用户空间栈需 U（用户 push）；内核空间栈不得带
    /// U——S 态 SUM=0 下访问 U 页会页故障，而内核任务跑 S 态。
    pub(crate) fn claim(
        &mut self,
        owner: usize,
        size: usize,
        kernel: bool,
    ) -> Result<VirtAddr, MapError> {
        // 槽 = 栈体 size + 守护页；栈窗 fall 排槽（自窗口顶向下，槽下方保持空闲）。
        let slot_size = size + TASK_STACK_GUARD;
        let (slot_va, _) = self
            .inner
            .allocator
            .allocate(slot_size, Direction::Fall)
            .map_err(|_| MapError::OutOfMemory)?;
        let slot_va = VirtAddr::from_raw(slot_va);
        // 守护页 Reserved → 溢出缺页可诊断
        self.inner.children.push(Map::new(
            slot_va,
            TASK_STACK_GUARD,
            PteFlags::V | PteFlags::R | PteFlags::W,
            MapKind::Reserved,
            Vec::new(),
            Some(owner),
        ));
        // 栈体 Anonymous，帧待 attach
        let body_flags = if kernel {
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D
        } else {
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D
        };
        let body_va = slot_va + TASK_STACK_GUARD;
        self.inner.children.push(Map::new(
            body_va,
            size,
            body_flags,
            MapKind::Anonymous,
            Vec::new(),
            Some(owner),
        ));
        Ok(body_va)
    }

    /// 按 owner 摘全部子 Map（守护页/栈体一并）并归还 slot 区间（退役/回滚共用）。
    ///
    /// 区间归还与子 Map 结构性绑定：disown 命中则 dealloc 必成功。返回被覆盖
    /// 区间 `[va, va + len)`（供调用方清 PTE）。
    pub(crate) fn reclaim(&mut self, owner: usize) -> Option<(VirtAddr, usize)> {
        let (va, len) = self.inner.disown(owner)?;
        let _ = self.inner.allocator.deallocate(va.as_usize(), len);
        Some((va, len))
    }
}