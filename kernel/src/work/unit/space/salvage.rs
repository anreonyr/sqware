// salvage — 拆除产出的待回收料（[`Span`] + [`Salvage`]）。
//!
//! 这两个类型是「分配/映射的产物 = 回收的输入」这条类型同一性的载体：
//! 窗口 claim/allocate/mmap 产出 [`Span`]，拆除时收进 [`Salvage`]，
//! 清退到齐后由 [`Salvage::reclaim`] 一次结清。
//!
//! 独立成文件的理由：它们是**回收侧**的词汇，与 [`SpaceInner`](super::inner::SpaceInner)
//! 的映射簿记是两件事；`Space`/`SpaceInner` 只提供结清所需的入口
//! （`asid()` / `with()` / `deallocate()`）。
//!
//! # 料箱必须零分配（不是省内存，是唯一能让释放路径不失败的办法）
//!
//! `release` 这条路上没有可返回的错误：调用方是 `reap.rs` 的 `.expect()` 与
//! `pole.rs` 的 `Denied`。故料箱不许往任何容器里推——摘下的图**自带一条链**
//! （[`Map::next`]），进箱只是把 `Box` 挂到链头（一次指针写）；一次性释放时
//! 至多一条 [`Span`]，故它是 `Option` 而不是 `Vec`。旧版这里是两个 `Vec`
//! （`maps: Vec<Map>` / `spans: Vec<Span>`），那正是拆除路径上那次
//! `RawVecInner::do_reserve_and_handle` 的来源（拆除路径上实测到的那次不可失败扩容）。

use alloc::boxed::Box;
use core::num::NonZeroUsize;

use super::outer::Space;
use super::map::Map;
use super::segment::SegmentKind;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::asid::{self, Deaf};

/// 一段 VA 区间 — 分配/映射动作的产物 = 回收的输入（类型同一）。
///
/// 由窗口 claim/allocate/mmap 产出，经 [`Space::release`] 回收。`pa` 仅对
/// **已物化固定帧**有意义（trap 帧恒 Some，栈/懒区恒 None）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    /// 所属段（回收定位）。
    pub(crate) seg: SegmentKind,
    /// 基址（页对齐）。
    pub(crate) va: VirtAddr,
    /// 总长（页对齐，非零）。
    pub(crate) size: NonZeroUsize,
    /// 物化帧物理地址（trap 帧恒 Some；栈/懒区恒 None）。
    pub(crate) pa: Option<PhysAddr>,
}

impl Span {
    pub(crate) fn new(seg: SegmentKind, va: VirtAddr, size: usize, pa: Option<PhysAddr>) -> Self {
        Self {
            seg,
            va,
            size: NonZeroUsize::new(size).expect("span size must be non-zero"),
            pa,
        }
    }
}

/// 拆除产出的待回收料 —— 摘下的图（自带链）与待还的段（[`Span`]）。
///
/// 硬不变量：**清退到齐之前不得易主**。帧归还即可被别的空间拿到、还段即 VA 可
/// 被本空间复用，而远核此刻可能仍持旧 TLB 条目 —— 两者都必须等在
/// [`Self::reclaim`] 里的清退之后。故 `Drop` 断言料箱已空（非空即被绕过）。
#[must_use = "salvage holds frames/segments that must be reclaimed after eviction"]
pub(crate) struct Salvage {
    /// 摘下的图链（帧随链上每张图 drop 归还 frame 池 / Arc 计数归零）。
    maps: Option<Box<Map>>,
    /// 待还的段区间（`release` 类拆除；`unmap` 类不还段则为空）——至多一条。
    span: Option<Span>,
}

impl Salvage {
    /// 空料箱（拆除事务前构造，事务内收料）。**不分配**。
    pub(crate) const fn new() -> Self {
        Self {
            maps: None,
            span: None,
        }
    }

    /// 收图（`SpaceInner::unmap` 调用）：挂到链头。**不分配**。
    ///
    /// 前置：这张图已从 `SpaceInner::maps` 摘除、`next` 为空（在册的图恒无链）。
    pub(super) fn take_map(&mut self, mut map: Box<Map>) {
        debug_assert!(map.next.is_none(), "salvage: 收进料箱的图不得已带链");
        map.next = self.maps.take();
        self.maps = Some(map);
    }

    /// 收段（拆除入口在校验通过后调用）。**不分配**。
    pub(super) fn take_span(&mut self, span: Span) {
        debug_assert!(self.span.is_none(), "salvage: 一次拆除至多一条 Span");
        self.span = Some(span);
    }

    /// 链上图的张数（`Drop` 的断言词用；**只在断言里读**，不做读数）。
    fn chain_len(&self) -> usize {
        let mut n = 0;
        let mut cur = self.maps.as_deref();
        while let Some(m) = cur {
            n += 1;
            cur = m.next.as_deref();
        }
        n
    }

    /// 结清：清退本空间 ASID → 还段 → 丢链（帧归还）。顺序即安全性。
    ///
    /// 空料箱直接返回（无易主 = 无清退义务），故装配回滚路径零成本。
    ///
    /// 前置：调用时**不得持 Space 锁**（本方法自行取锁还段；清退在锁外）。
    ///
    /// # Errors
    ///
    /// [`Deaf`] = RFENCE 清退失败（致命级，适配层裁定策略）。
    pub(crate) fn reclaim(mut self, space: &Space) -> Result<(), Deaf> {
        let maps = self.maps.take();
        let span = self.span.take();
        if maps.is_none() && span.is_none() {
            return Ok(());
        }
        asid::shootdown(space.asid())?;
        space.with(|inner| {
            if let Some(span) = span {
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
            self.maps.is_none() && self.span.is_none(),
            "salvage dropped unreclaimed: {} maps, {} spans",
            self.chain_len(),
            usize::from(self.span.is_some())
        );
    }
}
