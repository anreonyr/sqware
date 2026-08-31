// FrameWindow — 线程 trap 帧（窗口适配层：kernel 段上的帧领域策略）。
//
// 内核半区常量区 `[TEAM_FRAME_BASE, +SIZE)`，S-only（用户不可触碰）。
// 每帧 = 一页 VA + 一个物理帧 + 叶 PTE + 登记 map（Eager，claim 即物化）。
// 产物 Span 带 `pa`（restore 热路径直接取帧）。帧 VA 无固定地址——切换代码
// 经帧内 self_va 定位，是每帧可任意放置的前提。

use alloc::boxed::Box;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::{Frame, FrameState};

use super::super::{Seg, Space};
use super::super::core::Span;
use super::super::map::Map;

/// 帧窗口（零状态策略）。
pub(crate) struct FrameWindow;

impl FrameWindow {
    /// 领一个线程 trap 帧：kernel 段取一页 VA → 分配物理帧 → 装 PTE（S-only）→
    /// 登记 map（Eager）。返回 Span（`pa` = 帧物理地址，restore 直接取帧）。
    /// 帧元数据（内核切换信息 + 用户上下文）由调用方随后经 `pa` 填充。
    ///
    /// # Errors
    ///
    /// 段耗尽或物理帧耗尽 → [`MapError::OutOfMemory`]（回滚：清残留 PTE + 段归还）。
    pub(crate) fn claim(space: &Space) -> Result<Span, MapError> {
        space.with_flush(|inner| {
            let kernel = &mut inner.kernel;
            let va = kernel.allocate(PAGE_SIZE).map_err(|_| MapError::OutOfMemory)?;
            let va = VirtAddr::from_raw(va);
            let flags =
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D; // S-only
            // 类别 = Task：trap 帧属任务生命周期——关机必须归零（①）。
            let page: Frame = unsafe {
                Box::try_new_zeroed_in(crate::memory::allocator::fence::alloc_frame(crate::memory::allocator::fence::FrameClass::Task))
                    .map_err(|_| MapError::OutOfMemory)?
                    .assume_init()
            };
            let pa = PhysAddr::from_raw(page.as_ptr() as usize);
            if inner.root.map(va, pa, PAGE_SIZE, flags).is_err() {
                // 回滚：清残留 PTE + VA 退回段
                inner.root.unmap(va);
                inner.kernel.deallocate(va.as_usize(), PAGE_SIZE);
                return Err(MapError::OutOfMemory);
            }
            inner.maps.push(Map::new(
                va,
                PAGE_SIZE,
                flags,
                None, // Eager
                alloc::vec![FrameState::Owned(page)],
            ));
            Ok(Span::new(Seg::Kernel, va, PAGE_SIZE, Some(pa)))
        })
    }
}
