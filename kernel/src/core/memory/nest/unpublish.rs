//! 迁移前的逐页撤销：只撤这一页的 4KiB 映射。
//!
//! 与 `detach` / `close` 的差别在范围与后果：这里不退占用、不改 `ViewId`、
//! 活动形状、VA 与权限，也不把关系摘出索引，更不模拟一次 `detach`。
//! 只把压在这一页上的 PTE 与它的发布记录一起转入退休，等本 Nest 的 ASID
//! 确认之后才放掉承载。

use alloc::sync::Arc;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;

use super::state::NestState;
#[cfg(debug_assertions)]
use super::{Asid, identity::vacant};
use super::{Nest, NestError, View};

/// 下一次“准备阶段失败”的目标 ASID；空闲哨兵表示没装。
#[cfg(debug_assertions)]
static FAIL_PREPARE_ASID: AtomicUsize = AtomicUsize::new(vacant());

/// 让 `asid` 上下一次逐页撤销在准备阶段失败：它必须在任何 PTE 被撤下之前。
#[cfg(debug_assertions)]
pub(crate) fn fail_unpublish_prepare(asid: Asid) {
    assert!(crate::testing());
    FAIL_PREPARE_ASID
        .compare_exchange(vacant(), asid.get(), Ordering::Relaxed, Ordering::Relaxed)
        .expect("unpublish preparation injection already armed");
}

impl Nest {
    /// 撤下 `view` 在 `address` 上的 4KiB 映射，把它那一页的承载交给退休。
    ///
    /// - 内核空间的物理使用不由撤下本关系结清；全局条目不受 ASID 定向失效覆盖。
    ///   两类都不参与移交，当场 `Pinned`。
    /// - 先结清本 Nest 之前那次失效：旧 TLB 没确认就不再撤新的。
    /// - 在 `Nest → View` 同一个临界区里认这一页：只有真有一条发布记录才算这条
    ///   关系握着它；没有记录（活动洞或本来就没发布）就没有可撤的映射，当场
    ///   成功返回。真握着的话，它必须正是 `expected`——不是就在动任何 PTE 之前
    ///   当场断言，不先撤再发现。
    /// - 容量与大叶拆分都在撤下之前备齐，之后不再有可失败步骤。
    /// - 承载只在 shootdown 成功之后随退休一起放掉；失败时这一页的退休责任
    ///   留在本 Nest，由 `retry_invalidation` 继续。
    pub(in crate::core::memory) fn unpublish_page(
        self: &Arc<Self>,
        view: &Arc<View>,
        address: VirtAddr,
        expected: &Arc<Frame>,
    ) -> Result<(), NestError> {
        if self.asid().is_kernel() || view.is_global() {
            return Err(NestError::Pinned);
        }
        assert!(
            address.offset() == 0,
            "unpublish_page takes a page-aligned address"
        );
        assert!(
            view.reservation
                .nest()
                .is_some_and(|owner| core::ptr::eq(Arc::as_ptr(&owner), Arc::as_ptr(self))),
            "unpublish_page requires the view's own live nest"
        );
        self.finish_invalidation()?;

        let mut state = self.state.lock();
        if !state.invalidation.is_clean() {
            return Err(NestError::Shootdown);
        }
        let mut inner = view.state.lock();
        let page = view.page_index(address);
        let Some(at) = inner.published.iter().position(|(held, _)| *held == page) else {
            return Ok(());
        };
        if state.phase.is_closed() {
            return Err(NestError::Pinned);
        }
        // 身份在取出发布记录之后、任何 PTE 被撕下之前核。
        assert!(
            Arc::ptr_eq(&inner.published[at].1, expected),
            "the published page holds the frame this transfer handed over"
        );
        debug_assert!(
            inner.covers(address.as_usize(), address.as_usize()),
            "a published mapping lies inside a live range"
        );
        // 提前备容量与几何：这一页可能只是大叶里的一格，先拆成 4KiB 再撤。
        state
            .retired_frames
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        state
            .pending_views
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        {
            let NestState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            root.prepare_geometry(address, PAGE_SIZE, self.geometry)?;
            root.reserve_retired_tables(retired_tables)?;
        }
        #[cfg(debug_assertions)]
        if FAIL_PREPARE_ASID
            .compare_exchange(
                self.asid.get(),
                vacant(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return Err(NestError::OutOfMemory);
        }

        // 过了这里只剩下已经定好的状态转移：PTE、发布记录与承载一起转入退休责任。
        {
            let NestState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            root.unmap_prepared_geometry(address, PAGE_SIZE, self.geometry);
            root.reclaim_empty_tables(retired_tables);
        }
        state.pending_views.push(view.clone());
        let (_, frame) = inner.published.remove(at);
        state.retired_frames.push(frame);
        drop(inner);
        state.mark_dirty();
        drop(state);
        // 退休的承载只在这次确认之后才随 `finish_invalidation` 放掉。
        self.finish_invalidation()?;
        Ok(())
    }
}
