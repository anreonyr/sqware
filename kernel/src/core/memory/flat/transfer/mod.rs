//! 无映射移交：把同一个承载从一个合法逻辑页槽交进另一个空槽。
//!
//! 不复制数据、不分配数据帧、不碰 PTE/TLB，也不改 Flat 身份、VA 与权限。
//! 两端都必须是普通 `acquire` 来源（`from_frames` 与借用来源拒），
//! 该页的承载还必须只归一个来源槽（被 `from_frames` 登记过就不再可迁移）。
//! 两端页只要有 `Residency`（例如一段 `View` 的整窗驻留）就当场 `Busy`：
//! 排除是当场核的，不是调用方的承诺。
//!
//! 同一份来源只取一次锁；不同来源按地址序共持一对（`machine::sync::with_pair`），
//! 于是源 `Resident → Missing` 与目标 `Fresh|Missing → Resident` 在同一个临界区里完成，
//! 中间没有可被看到的窗口。

mod error;

pub use error::TransferError;

use alloc::sync::Arc;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::core::memory::address::{PageCount, PageIndex, PageOffset};
use crate::machine::sync::with_pair;

use super::Flat;
use super::FlatRange;
use super::Frame;
use super::slot::{self, Slot};
use super::source::FlatSource;
use super::state::FlatState;

#[cfg(debug_assertions)]
static FAIL_PREPARE: AtomicBool = AtomicBool::new(false);

/// 下一次移交的准备阶段（目标槽容量）失败：它必须在任何槽被改动之前。
#[cfg(debug_assertions)]
pub(crate) fn fail_transfer_prepare() {
    assert!(crate::testing());
    FAIL_PREPARE.store(true, Ordering::Relaxed);
}

/// 一次移交：两端各自的页号是相对各自窗口的偏移，换成本来源上的绝对页号。
pub(crate) struct Transfer {
    source: Arc<Flat>,
    source_page: PageIndex,
    target: Arc<Flat>,
    target_page: PageIndex,
}

impl Transfer {
    /// 换两端绝对页号：两端各自显式检查范围；同一来源上的同一页当场拒。
    pub(crate) fn new(
        source: FlatRange,
        source_page: PageOffset,
        target: FlatRange,
        target_page: PageOffset,
    ) -> Result<Self, TransferError> {
        let source_page = page_at(&source, source_page)?;
        let target_page = page_at(&target, target_page)?;
        if Arc::ptr_eq(source.flat(), target.flat()) && source_page == target_page {
            return Err(TransferError::DestinationOccupied);
        }
        Ok(Self {
            source: source.flat().clone(),
            source_page,
            target: target.flat().clone(),
            target_page,
        })
    }

    /// 执行移交。
    pub(crate) fn execute(&self) -> Result<(), TransferError> {
        if Arc::ptr_eq(&self.source, &self.target) {
            let mut state = self.source.state.lock();
            check_source(&self.source, &state, self.source_page)?;
            check_target(&self.target, &state, self.target_page)?;
            reserve(&mut state, self.target_page)?;
            let frame = carve(&mut state, self.source_page)?;
            place(&mut state, self.target_page, frame.clone());
            frame.custody().leave();
            Ok(())
        } else {
            with_pair(&self.source.state, &self.target.state, |from, to| {
                check_source(&self.source, from, self.source_page)?;
                check_target(&self.target, to, self.target_page)?;
                reserve(to, self.target_page)?;
                let frame = carve(from, self.source_page)?;
                place(to, self.target_page, frame.clone());
                frame.custody().leave();
                Ok(())
            })
        }
    }
}

/// 选端的显式范围检查：偏移必须落在本窗口内，换成来源上的绝对页号。
fn page_at(range: &FlatRange, offset: PageOffset) -> Result<PageIndex, TransferError> {
    if offset.get() >= range.page_count().get() {
        return Err(TransferError::OutOfRange);
    }
    let page = range
        .index()
        .get()
        .checked_add(offset.get())
        .ok_or(TransferError::OutOfRange)?;
    Ok(PageIndex::new(page))
}

/// 两端共同的来源资格：普通 `acquire` 建立的 RAM 来源。
fn plain_origin(flat: &Flat, state: &FlatState) -> Result<(), TransferError> {
    if flat.imported || !matches!(state.source, FlatSource::Managed(_)) {
        return Err(TransferError::NotTransferable);
    }
    Ok(())
}

/// 这一页是否被驻留覆盖：两端都一样，覆盖就当场 Busy。
fn pinned(state: &FlatState, page: PageIndex) -> bool {
    state.is_pinned(page, PageCount::new(1).expect("one page"))
}

/// 源端：普通来源、无驻留，且这一页真的有承载。
fn check_source(flat: &Flat, state: &FlatState, page: PageIndex) -> Result<(), TransferError> {
    plain_origin(flat, state)?;
    if pinned(state, page) {
        return Err(TransferError::Busy);
    }
    match carrier(state, page) {
        Some(_) => Ok(()),
        None => Err(TransferError::SourceEmpty),
    }
}

/// 目标端：普通来源、无驻留，且这一页是空的（Fresh 或 `Missing` 墓碑）。
fn check_target(flat: &Flat, state: &FlatState, page: PageIndex) -> Result<(), TransferError> {
    plain_origin(flat, state)?;
    if pinned(state, page) {
        return Err(TransferError::Busy);
    }
    let FlatSource::Managed(slots) = &state.source else {
        return Err(TransferError::NotTransferable);
    };
    let (at, present) = slot::spot(slots, page);
    if present && matches!(slots[at].1, Slot::Resident(_)) {
        return Err(TransferError::DestinationOccupied);
    }
    Ok(())
}

fn carrier(state: &FlatState, page: PageIndex) -> Option<&Arc<Frame>> {
    let FlatSource::Managed(slots) = &state.source else {
        return None;
    };
    let (at, present) = slot::spot(slots, page);
    if !present {
        return None;
    }
    match &slots[at].1 {
        Slot::Resident(frame) => Some(frame),
        Slot::Missing => None,
    }
}

/// 目标槽的容量：在取得承载独占**之前**备齐，于是 OOM 不会留下 `InTransit`。
fn reserve(state: &mut FlatState, page: PageIndex) -> Result<(), TransferError> {
    #[cfg(debug_assertions)]
    if FAIL_PREPARE.swap(false, Ordering::Relaxed) {
        return Err(TransferError::OutOfMemory);
    }
    let FlatSource::Managed(slots) = &mut state.source else {
        return Err(TransferError::NotTransferable);
    };
    let (_, present) = slot::spot(slots, page);
    if present {
        return Ok(());
    }
    slots
        .try_reserve(1)
        .map_err(|_| TransferError::OutOfMemory)?;
    Ok(())
}

/// 取走源槽的承载：`Resident → Missing`，并且先取得它的移交独占。
fn carve(state: &mut FlatState, page: PageIndex) -> Result<Arc<Frame>, TransferError> {
    let FlatSource::Managed(slots) = &mut state.source else {
        return Err(TransferError::NotTransferable);
    };
    let (at, present) = slot::spot(slots, page);
    if !present {
        return Err(TransferError::SourceEmpty);
    }
    let frame = match &slots[at].1 {
        Slot::Resident(frame) => frame.clone(),
        Slot::Missing => return Err(TransferError::SourceEmpty),
    };
    if !frame.custody().enter() {
        // 只在取独占失败后才选错误码：`Aliased` 是永久拒绝，`InTransit` 是可重试。
        return Err(if frame.custody().is_aliased() {
            TransferError::NotTransferable
        } else {
            TransferError::Busy
        });
    }
    slots[at].1 = Slot::Missing;
    Ok(frame)
}

/// 承载落进目标槽：替换已有的墓碑，或插入新槽。
fn place(state: &mut FlatState, page: PageIndex, frame: Arc<Frame>) {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("both ends are plain acquired sources, checked before the move")
    };
    let (at, present) = slot::spot(slots, page);
    if present {
        slots[at].1 = Slot::Resident(frame);
    } else {
        slots.insert(at, (page, Slot::Resident(frame)));
    }
}
