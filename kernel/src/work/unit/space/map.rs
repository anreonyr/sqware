// Map — VA→PA 簿记的原子单元（簿记模型第三层）。
//
// 语义：本映射覆盖 VA 区间 `[va, va + size)`；`frames[i]` 是 VA
// `va + i·PAGE_SIZE` 处物理帧的持有者（**不变量**：帧 i ↔ va + i·PAGE_SIZE）。
// 借用映射（DRAM 恒等、trampoline 叶：物理帧归内核/机器）frames 为空，PA 由
// 页表维护；拥有映射（文本、trap-context、堆块、栈体）帧随 Map drop 归还 frame
// 池——所有权即回收，无遍历页表树、无手写 deallocate。

use core::num::NonZeroUsize;

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::{Frame, FrameState, MapError};

/// 映射种类 — 缺页时如何响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapKind {
    /// 匿名映射 — 缺页时分配零页
    Anonymous,
    /// 预留映射 — 不可访问，缺页时返回错误
    ///
    /// 任务栈守护页与内核借用映射（DRAM 恒等、trampoline 叶）以 Reserved 登记：
    /// 越权触碰时返回「预留映射访问」而非笼统的「无 Map」。
    Reserved,
}

/// 虚拟→物理映射 — 簿记的原子单元。
///
/// 语义：本映射覆盖 VA 区间 `[va, va + size)`；`frames[i]` 是 VA
/// `va + i·PAGE_SIZE` 处物理帧的持有者（**不变量**：帧 i ↔ va + i·PAGE_SIZE）。
/// 借用映射（DRAM 恒等、trampoline 叶：物理帧归内核/机器）frames 为空，PA 由
/// 页表维护；拥有映射（文本、trap-context、堆块、栈体）帧随 Map drop 归还 frame
/// 池——所有权即回收，无遍历页表树、无手写 deallocate。
#[derive(Debug)]
pub struct Map {
    pub(super) va: VirtAddr,
    pub(super) size: NonZeroUsize,
    pub(super) flags: PteFlags,
    pub(super) kind: MapKind,
    /// 所属线程 id（线程私有映射：栈 guard/体、trap 帧）——退出时按 owner 定位回收；
    /// 共享/空间级映射（文本、DRAM、trampoline、堆块）为 `None`。
    pub(super) owner: Option<usize>,
    pub(super) frames: Vec<FrameState>,
}

impl Map {
    /// 构造（size 必须非零——调用方保证，见各入口的校验）。
    pub(super) fn new(
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        kind: MapKind,
        frames: Vec<FrameState>,
        owner: Option<usize>,
    ) -> Self {
        Self {
            va,
            size: NonZeroUsize::new(size).expect("map size must be non-zero"),
            flags,
            kind,
            owner,
            frames,
        }
    }

    /// 是否覆盖 `vaddr`（减法判定，避免最高页 `va + size` 溢出）。
    pub(super) fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    /// 偏移 `off`（页内偏移无关，按页取帧）处的物理地址。
    ///
    /// # Errors
    ///
    /// 越界（`off >= size`）或该页尚无帧（懒分配未就位）→ [`MapError::NotMapped`]。
    /// munmap/mprotect 后端预留：按 VA→PA 语义直接取帧 PA
    pub(super) fn translate(&self, off: usize) -> Result<PhysAddr, MapError> {
        if off >= self.size.get() {
            return Err(MapError::NotMapped);
        }
        let idx = off / PAGE_SIZE;
        let frame = self.frames.get(idx).ok_or(MapError::NotMapped)?;
        Ok(frame.pa())
    }

    /// 注入一帧（保持不变量；调用方保证序号连续且不越界）。
    pub(super) fn inject(&mut self, frame: Frame) {
        debug_assert!(
            self.frames.len() < self.size.get() / PAGE_SIZE,
            "map {:#x} frame overflow",
            self.va.as_usize()
        );
        self.frames.push(FrameState::Owned(frame));
    }
}
