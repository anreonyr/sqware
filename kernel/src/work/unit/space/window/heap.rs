// HeapWindow — 用户堆（窗口适配层：user 段上的堆领域策略，立即物化）。
//
// allocate 出块：user 段取段 + `claim`（Eager，立即分配非懒）+ 登记 map。
// deallocate 精确匹配释放：还段 + 统一拆除（摘 map 帧随 drop 归还 + 清叶 PTE）。
// 与栈/懒区同池取段（user 段 lowest first-fit），无方向分区。

use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::core::SpaceInner;
use super::super::salvage::Span;
use super::super::{Seg, Space};

/// 堆窗口（零状态策略）。
pub(crate) struct HeapWindow;

impl HeapWindow {
    /// 用户堆分配：user 段取一页对齐 VA 块，逐页从 frame 分配器取物理页
    /// 映射（R|W，**立即分配**非懒）并注入 map。返回 Span。
    ///
    /// U 位经 [`Space::pte_policy`] 单一出口：U 态页表需 U；S 态页表 SUM=0，不得
    /// 带 U（否则域任务一用堆就缺页）。
    ///
    /// 中途帧耗尽时回滚：`claim` 自清已映射页叶 + 摘 map，本方法退 VA 回段。
    ///
    /// # Errors
    ///
    /// 段耗尽 / 物理帧耗尽 → [`MapError::OutOfMemory`]。
    pub(crate) fn allocate(space: &Space, size: usize) -> Result<Span, MapError> {
        let flags =
            space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        space.with_flush(|inner| {
            let va = inner.allocate(Seg::User, size)?;
            // 种类 = Heap：用户堆的物理页。
            let next = || Ok(crate::tag!(Heap, SpaceInner::frame()?));
            if let Err(e) = inner.claim(va, size, flags, next) {
                // claim 已自回滚装配；段退回
                inner.deallocate(Seg::User, va.as_usize(), size);
                return Err(e);
            }
            Ok(Span::new(Seg::User, va, size, None))
        })
    }

    /// 用户堆释放：按 `(addr, size)` 精确匹配**校验**段 → 统一拆除（清叶 PTE +
    /// 摘 map，帧交料箱）→ 结清（跨核清退到齐后才还段与帧）。返回是否找到并释放。
    ///
    /// # 本方法不再自己拆装
    ///
    /// 此前这里把 `Space::release` 的整段流程（`holds` + `unmap` + `take_span` +
    /// `reclaim`）**复制了一遍**，与 `ShareWindow::munmap` 逐字同构——同一件事
    /// 三份实现、三套失败语义。收敛后只剩一层：校验与还原走
    /// [`Space::release_addr`]，拆装与结清只有 `Space::release` 一处。
    ///
    /// 语义不变：该区间不是本段的已分配块 → `false`（状态未动）。
    pub(crate) fn deallocate(space: &Space, addr: VirtAddr, size: usize) -> bool {
        space.release_addr(Seg::User, addr, size)
    }
}
