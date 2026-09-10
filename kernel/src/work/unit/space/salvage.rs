// salvage — 拆除产出的待回收料（[`Span`] + [`Salvage`]）。
//!
//! 这两个类型是「分配/映射的产物 = 回收的输入」这条类型同一性的载体：
//! 窗口 claim/allocate/mmap 产出 [`Span`]，拆除时收进 [`Salvage`]，
//! 清退到齐后由 [`Salvage::reclaim`] 一次结清。
//!
//! 独立成文件的理由：它们是**回收侧**的词汇，与 [`SpaceInner`](super::inner::SpaceInner)
//! 的映射簿记是两件事；`Space`/`SpaceInner` 只提供结清所需的入口
//! （`asid()` / `with()` / `deallocate()`）。

use alloc::vec::Vec;
use core::num::NonZeroUsize;

use super::adapter::Space;
use super::map::Map;
use super::seg::Seg;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::asid::{self, Deaf};

/// 一段 VA 区间 — 分配/映射动作的产物 = 回收的输入（类型同一）。
///
/// 由窗口 claim/allocate/mmap 产出，经 [`Space::release`] 回收。`pa` 仅对
/// **已物化固定帧**有意义（trap 帧恒 Some，栈/懒区恒 None）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    /// 所属段（回收定位）。
    pub(crate) seg: Seg,
    /// 基址（页对齐）。
    pub(crate) va: VirtAddr,
    /// 总长（页对齐，非零）。
    pub(crate) size: NonZeroUsize,
    /// 物化帧物理地址（trap 帧恒 Some；栈/懒区恒 None）。
    pub(crate) pa: Option<PhysAddr>,
}

impl Span {
    pub(crate) fn new(seg: Seg, va: VirtAddr, size: usize, pa: Option<PhysAddr>) -> Self {
        Self {
            seg,
            va,
            size: NonZeroUsize::new(size).expect("span size must be non-zero"),
            pa,
        }
    }
}

/// 拆除产出的待回收料 —— 摘下的帧（整张 [`Map`]）与待还的段（[`Span`]）。
///
/// 硬不变量：**清退到齐之前不得易主**。帧归还即可被别的空间拿到、还段即 VA 可
/// 被本空间复用，而远核此刻可能仍持旧 TLB 条目 —— 两者都必须等在
/// [`Self::reclaim`] 里的清退之后。故 `Drop` 断言料箱已空（非空即被绕过）。
#[must_use = "salvage holds frames/segments that must be reclaimed after eviction"]
pub(crate) struct Salvage {
    /// 摘下的映射（帧随其 drop 归还 frame 池 / Arc 计数归零）。
    maps: Vec<Map>,
    /// 待还的段区间（`release` 类拆除；`unmap` 类不还段则为空）。
    spans: Vec<Span>,
}

impl Salvage {
    /// 空料箱（拆除事务前构造，事务内收料）。
    pub(crate) const fn new() -> Self {
        Self {
            maps: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// 收帧（`SpaceInner::unmap` / `Map::carve` 调用）。
    pub(super) fn take_map(&mut self, map: Map) {
        self.maps.push(map);
    }

    /// 收段（拆除入口在校验通过后调用）。
    pub(super) fn take_span(&mut self, span: Span) {
        self.spans.push(span);
    }

    /// 结清：清退本空间 ASID → 还段 → 帧 drop。顺序即安全性。
    ///
    /// 空料箱直接返回（无易主 = 无清退义务），故装配回滚路径零成本。
    ///
    /// 前置：调用时**不得持 Space 锁**（本方法自行取锁还段；清退在锁外）。
    ///
    /// # Errors
    ///
    /// [`Deaf`] = RFENCE 清退失败（致命级，适配层裁定策略）。
    pub(crate) fn reclaim(mut self, space: &Space) -> Result<(), Deaf> {
        let maps = core::mem::take(&mut self.maps);
        let spans = core::mem::take(&mut self.spans);
        if maps.is_empty() && spans.is_empty() {
            return Ok(());
        }
        asid::shootdown(space.asid())?;
        space.with(|inner| {
            for span in &spans {
                let ok = inner.deallocate(span.seg, span.va.as_usize(), span.size.get());
                debug_assert!(ok, "salvage: segment mismatch on reclaim {:?}", span.va);
            }
        });
        drop(maps);
        Ok(())
    }
}

impl Drop for Salvage {
    fn drop(&mut self) {
        debug_assert!(
            self.maps.is_empty() && self.spans.is_empty(),
            "salvage dropped unreclaimed: {} maps, {} spans",
            self.maps.len(),
            self.spans.len()
        );
    }
}
