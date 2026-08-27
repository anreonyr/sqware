// FrameWindow — team 帧区窗口（动态侧：线程 trap 帧）。
//
// 内核半区常量区 `[TEAM_FRAME_BASE, TEAM_FRAME_BASE + TEAM_FRAME_WINDOW_SIZE)`，
// S-only（用户不可触碰）。每项 = 一页 VA + 一个物理帧 + 叶 PTE + 子 Map
// （owner = 线程 id）：claim 即物化（帧当场分配并映射）；线程退役经 reclaim 回收。
// 帧 VA 无固定地址——切换代码经帧内 self_va 定位，是每线程帧可任意放置的前提。

use alloc::boxed::Box;

use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::Frame;
use crate::memory::manager::MapError;

use super::super::durable::Durable;
use super::super::dynamic::Dynamic;
use super::super::map::MapKind;

/// 帧区窗口。
#[derive(Debug)]
pub(crate) struct FrameWindow {
    /// 公共窗口核心（区间分配器 + 子 Map 表）。
    pub(crate) inner: Dynamic,
}

impl FrameWindow {
    /// 构造：内核半区常量区 `[TEAM_FRAME_BASE, +TEAM_FRAME_WINDOW_SIZE)`。
    pub(crate) fn new() -> Self {
        Self {
            inner: Dynamic::window(
                TEAM_FRAME_BASE.as_usize(),
                TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE,
            ),
        }
    }

    /// 领一个线程 trap 帧：窗口取一页 VA → 分配物理帧 → 装 PTE（S-only）→ 登记帧
    /// 子 Map（owner = 线程 id）。返回 `(帧 VA, 帧 PA)`；帧元数据（内核切换信息 +
    /// 用户上下文）由调用方随后经 PA 填充。
    ///
    /// # Errors
    ///
    /// 窗口耗尽或物理帧耗尽 → [`MapError::OutOfMemory`]（后者回滚：清残留 PTE +
    /// VA 退回窗口，空子 Map 一并移除）。
    pub(crate) fn claim(
        &mut self,
        durable: &mut Durable,
        owner: usize,
    ) -> Result<(VirtAddr, PhysAddr), MapError> {
        let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D; // S-only
        let va = self
            .inner
            .allocate(PAGE_SIZE, flags, MapKind::Anonymous, Some(owner))?;
        let page: Frame = unsafe {
            Box::try_new_zeroed_in(allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        };
        let pa = PhysAddr::from_raw(page.as_ptr() as usize);
        if durable.root.map(va, pa, PAGE_SIZE, flags).is_err() {
            // 回滚：清可能残留的中间表/PTE + VA 退回窗口
            durable.unmap_frames(va, PAGE_SIZE);
            self.inner.deallocate(va, PAGE_SIZE);
            return Err(MapError::OutOfMemory);
        }
        let child = self
            .inner
            .children
            .iter_mut()
            .find(|m| m.va == va)
            .expect("frame child exists");
        child.inject(page);
        Ok((va, pa))
    }

    /// 退役回收：精确摘帧 VA（子 Map 移除，帧随 drop 归还 frame 池）+ 区间树归还。
    /// 返回是否命中（未分配/已回收 → false）。PTE 清理由调用方经 `unmap_frames` 完成。
    pub(crate) fn reclaim(&mut self, va: VirtAddr) -> bool {
        self.inner.deallocate(va, PAGE_SIZE)
    }
}