// FrameWindow — 线程 trap 帧（窗口适配层：kernel 段上的帧领域策略）。
//
// 内核半区常量区 `[TEAM_FRAME_BASE, +SIZE)`，S-only（用户不可触碰）。
// 每帧 = 一页 VA + 一个物理帧 + 叶 PTE + 登记 map（`claim`，立即物化）。
// 产物 Span 带 `pa`（restore 热路径直接取帧）。帧 VA 无固定地址——切换代码
// 经帧内 self_va 定位，是每帧可任意放置的前提。

use super::super::core::Span;
use super::super::{Seg, Space};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::entry::PteFlags;

/// 帧窗口（零状态策略）。
pub(crate) struct FrameWindow;

impl FrameWindow {
    /// 领一个线程 trap 帧：kernel 段取一页 VA → 分配物理帧 → 装 PTE（S-only）→
    /// 登记 map（`claim`，立即物化）。返回 Span（`pa` = 帧物理地址，restore
    /// 直接取帧）。帧元数据（内核切换信息 + 用户上下文）由调用方随后经 `pa`
    /// 填充。
    ///
    /// # Errors
    ///
    /// 段耗尽或物理帧耗尽 → [`MapError::OutOfMemory`]（回滚：段归还）。
    pub(crate) fn claim(space: &Space) -> Result<Span, MapError> {
        space.with_flush(|inner| {
            let va = inner.allocate(Seg::Kernel, PAGE_SIZE)?;
            let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D; // S-only
            if let Err(e) = inner.claim(va, PAGE_SIZE, flags) {
                // claim 已自回滚装配；段退回
                inner.deallocate(Seg::Kernel, va.as_usize(), PAGE_SIZE);
                return Err(e);
            }
            let pa = inner.translate(va).expect("frame claimed").0;
            Ok(Span::new(Seg::Kernel, va, PAGE_SIZE, Some(pa)))
        })
    }
}
