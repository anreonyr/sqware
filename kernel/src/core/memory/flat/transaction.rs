//! 一次移交的短事务：两端页槽在同一个临界区里进入在途态。
//!
//! 事务只改页槽，不复制数据、不分配数据帧、不碰 PTE/TLB。
//! 守卫活着的时候两端就一直是那个在途态：源头是 `Moving`（自己强持原 `Frame`），
//! 目标是 `Reserved`（记着原本是 Fresh 还是 `Missing`）。于是提前返回、`?` 或
//! panic=abort 都不会让承载失去归属——它始终在源槽里。到点 `commit` 消费守卫，
//! 没 commit 就被 Drop 的按同一笔身份原样回退两端。
//!
//! 同一份来源只取一次锁；不同来源按地址序共持一对（`machine::sync::with_pair`）。
//! 可失败的步骤（两端资格、两端驻留与在途、目标槽容量，以及在这之后、写槽之前的
//! 纯 core 准备回调）全在任何槽被改动之前做完；随后只有承载独占与写槽，写槽不再
//! 返回失败。

use alloc::sync::Arc;
use core::sync::atomic::AtomicUsize;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::core::memory::address::{PageCount, PageIndex};
use crate::core::memory::transfer::TransferError;
use crate::machine::sync::with_pair;

use super::slot::{self, Slot};
use super::source::FlatSource;
use super::state::FlatState;
use super::{Flat, FlatRange, Frame};

#[cfg(debug_assertions)]
static FAIL_PREPARE: AtomicBool = AtomicBool::new(false);

/// 下一次移交的准备阶段（目标槽容量）失败：它必须在任何槽被改动之前。
#[cfg(debug_assertions)]
pub(crate) fn fail_transfer_prepare() {
    assert!(crate::testing());
    FAIL_PREPARE.store(true, Ordering::Relaxed);
}

/// 一次移交：两端各自的页已经换成本来源上的绝对页号。
pub(in crate::core::memory) struct Transaction {
    source: Arc<Flat>,
    source_page: PageIndex,
    target: Arc<Flat>,
    target_page: PageIndex,
    id: usize,
    committed: bool,
}

impl Transaction {
    /// 短双锁里占槽：源 `Resident → Moving`，目标 `Fresh | Missing → Reserved`。
    ///
    /// 两端资格、pin、页槽与目标容量这些可失败的检查全做完之后、任何 `Moving`/
    /// `Reserved` 槽或承载独占被写之前，调一次纯 core 的 `prepare`；它自己可失败，
    /// 失败时守卫尚未构造、也没有一格槽被改动。成功后守卫才连同 `prepare` 的载荷
    /// 一起交出，于是占槽与它引用的元数据来自同一个临界区。
    pub(in crate::core::memory) fn claim<C>(
        source: &FlatRange,
        source_page: PageIndex,
        target: &FlatRange,
        target_page: PageIndex,
        prepare: impl FnOnce() -> Result<C, TransferError>,
    ) -> Result<(Self, C), TransferError> {
        assert!(
            owns(source, source_page),
            "the source page is inside its window"
        );
        assert!(
            owns(target, target_page),
            "the target page is inside its window"
        );
        let same = Arc::ptr_eq(source.flat(), target.flat());
        if same && source_page == target_page {
            return Err(TransferError::DestinationOccupied);
        }
        let id = next_transaction()?;
        let source = source.flat().clone();
        let target = target.flat().clone();
        let carried = if same {
            let mut state = source.state.lock();
            source_ready(&state, source_page)?;
            let was_missing = target_ready(&state, target_page)?;
            reserve(&mut state, target_page)?;
            let carried = prepare()?;
            claim_source(&mut state, source_page, id)?;
            claim_target(&mut state, target_page, id, was_missing);
            carried
        } else {
            with_pair(&source.state, &target.state, |from, to| {
                source_ready(from, source_page)?;
                let was_missing = target_ready(to, target_page)?;
                reserve(to, target_page)?;
                let carried = prepare()?;
                claim_source(from, source_page, id)?;
                claim_target(to, target_page, id, was_missing);
                Ok(carried)
            })?
        };
        let transaction = Self {
            source,
            source_page,
            target,
            target_page,
            id,
            committed: false,
        };
        Ok((transaction, carried))
    }

    /// 结算并消费守卫：源成墓碑，目标拿到同一个承载；之后 Drop 不再回退。
    pub(in crate::core::memory) fn commit(mut self) {
        self.settle();
        self.committed = true;
    }

    /// 源槽此刻强持的那个承载：只读提取，不改归属、不放独占。
    ///
    /// 调用方拿它核对“撤下来的那一页正是本次要移交的同一份帧”，
    /// 而不是再存一份 PA 或第二个归属权威。
    pub(in crate::core::memory) fn carrier(&self) -> Arc<Frame> {
        let state = self.source.state.lock();
        let FlatSource::Managed(slots) = &state.source else {
            unreachable!("the source was a plain acquired source when it was claimed")
        };
        let (at, present) = slot::spot(slots, self.source_page);
        assert!(present, "a claimed source slot keeps its seat");
        match &slots[at].1 {
            Slot::Moving { frame, transaction } => {
                assert!(
                    *transaction == self.id,
                    "the source slot belongs to this transaction"
                );
                frame.clone()
            }
            _ => panic!("a claimed source slot stays Moving"),
        }
    }

    /// 两端在同一临界区里结算：`Moving → Missing`，`Reserved → Resident`。
    fn settle(&self) {
        if Arc::ptr_eq(&self.source, &self.target) {
            let mut state = self.source.state.lock();
            let frame = settle_source(&mut state, self.source_page, self.id);
            settle_target(&mut state, self.target_page, self.id, frame);
        } else {
            with_pair(&self.source.state, &self.target.state, |from, to| {
                let frame = settle_source(from, self.source_page, self.id);
                settle_target(to, self.target_page, self.id, frame);
            });
        }
    }
}

impl Drop for Transaction {
    /// 没有提交就被放掉：两端在同一笔身份下原样还回去。
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if Arc::ptr_eq(&self.source, &self.target) {
            let mut state = self.source.state.lock();
            rollback_source(&mut state, self.source_page, self.id);
            rollback_target(&mut state, self.target_page, self.id);
        } else {
            with_pair(&self.source.state, &self.target.state, |from, to| {
                rollback_source(from, self.source_page, self.id);
                rollback_target(to, self.target_page, self.id);
            });
        }
    }
}

/// 页落在窗口里：入口已经查过，这里是不变量。
fn owns(range: &FlatRange, page: PageIndex) -> bool {
    page.get() >= range.index().get() && page.get() - range.index().get() < range.page_count().get()
}

/// 一次移交的事务身份：同一笔的两端一致，用来认自己的在途槽。
///
/// 号耗尽（或超出符号上界）就当场失败，不回收旧号。
fn next_transaction() -> Result<usize, TransferError> {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    crate::machine::sync::next_id(&NEXT).ok_or(TransferError::OutOfMemory)
}

/// 两端共同的来源资格：这一份来源还允许迁移。
///
/// 资格是来源自己的**永久**状态，不靠“现在有没有关系压着它”现算：导入、借用，
/// 以及任何一次内核或全局发布都会把它整份置假——全局条目对 ASID 定向失效免疫，
/// 旧 TLB 无从确认，而关联退场并不能证明它没了。
fn plain_origin(state: &FlatState) -> Result<(), TransferError> {
    if !state.transferable {
        return Err(TransferError::NotTransferable);
    }
    Ok(())
}

/// 这一页是否被驻留覆盖：两端都一样，覆盖就当场 Busy。
fn pinned(state: &FlatState, page: PageIndex) -> bool {
    state.is_pinned(page, PageCount::new(1).expect("one page"))
}

/// 源端：来源仍可迁、无驻留，且这一页真的是空闲的承载。
fn source_ready(state: &FlatState, page: PageIndex) -> Result<(), TransferError> {
    plain_origin(state)?;
    if pinned(state, page) {
        return Err(TransferError::Busy);
    }
    let FlatSource::Managed(slots) = &state.source else {
        return Err(TransferError::NotTransferable);
    };
    let (at, present) = slot::spot(slots, page);
    if !present {
        return Err(TransferError::SourceEmpty);
    }
    match slots[at].1 {
        Slot::Resident(_) => Ok(()),
        Slot::Missing => Err(TransferError::SourceEmpty),
        Slot::Moving { .. } | Slot::Reserved { .. } => Err(TransferError::Busy),
    }
}

/// 目标端：来源仍可迁、无驻留，且这一页是空的（Fresh 或 `Missing` 墓碑）。
///
/// 返回值是“目标原本是不是墓碑”：回退时要原样还回去，不能把墓碑抹成 Fresh。
fn target_ready(state: &FlatState, page: PageIndex) -> Result<bool, TransferError> {
    plain_origin(state)?;
    if pinned(state, page) {
        return Err(TransferError::Busy);
    }
    let FlatSource::Managed(slots) = &state.source else {
        return Err(TransferError::NotTransferable);
    };
    let (at, present) = slot::spot(slots, page);
    if !present {
        return Ok(false);
    }
    match slots[at].1 {
        Slot::Resident(_) => Err(TransferError::DestinationOccupied),
        Slot::Missing => Ok(true),
        Slot::Moving { .. } | Slot::Reserved { .. } => Err(TransferError::Busy),
    }
}

/// 目标槽的容量：在取得承载独占**之前**备齐，于是 OOM 不会留下在途槽。
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
    slots.try_reserve(1).map_err(|_| TransferError::OutOfMemory)
}

/// 取走源槽的承载：`Resident → Moving`（事务身份 + 帧留在槽里），并取得移交独占。
fn claim_source(state: &mut FlatState, page: PageIndex, id: usize) -> Result<(), TransferError> {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("the source was checked to be a plain acquired source")
    };
    let (at, present) = slot::spot(slots, page);
    assert!(present, "the source page was checked to hold a carrier");
    let frame = match &slots[at].1 {
        Slot::Resident(frame) => frame.clone(),
        _ => unreachable!("the source page was checked to be an idle carrier"),
    };
    if !frame.custody().enter() {
        // 只在取独占失败后才选错误码：`Aliased` 是永久拒结，`InTransit` 是可重试。
        return Err(if frame.custody().is_aliased() {
            TransferError::NotTransferable
        } else {
            TransferError::Busy
        });
    }
    slots[at].1 = Slot::Moving {
        frame,
        transaction: id,
    };
    Ok(())
}

/// 目标槽进保留：`Fresh | Missing → Reserved`（容量已在 `reserve` 备好）。
fn claim_target(state: &mut FlatState, page: PageIndex, id: usize, was_missing: bool) {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("the target was checked to be a plain acquired source")
    };
    let (at, present) = slot::spot(slots, page);
    if present {
        assert!(
            was_missing && matches!(slots[at].1, Slot::Missing),
            "an occupied target slot is exactly the tombstone that was checked"
        );
        slots[at].1 = Slot::Reserved {
            transaction: id,
            was_missing,
        };
        return;
    }
    assert!(!was_missing, "an absent target slot is fresh");
    slots.insert(
        at,
        (
            page,
            Slot::Reserved {
                transaction: id,
                was_missing: false,
            },
        ),
    );
}

/// 结算源槽：`Moving → Missing`；交出来的正是这一笔占住的那个帧。
fn settle_source(state: &mut FlatState, page: PageIndex, id: usize) -> Arc<Frame> {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("the source was a plain acquired source when it was claimed")
    };
    let (at, present) = slot::spot(slots, page);
    assert!(present, "a claimed source slot keeps its seat");
    let frame = match &slots[at].1 {
        Slot::Moving { frame, transaction } => {
            assert!(
                *transaction == id,
                "the source slot belongs to this transaction"
            );
            frame.clone()
        }
        _ => panic!("a claimed source slot stays Moving"),
    };
    slots[at].1 = Slot::Missing;
    frame
}

/// 结算目标槽：`Reserved → Resident`；承载的独占在这里还回去。
fn settle_target(state: &mut FlatState, page: PageIndex, id: usize, frame: Arc<Frame>) {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("the target was a plain acquired source when it was claimed")
    };
    let (at, present) = slot::spot(slots, page);
    assert!(present, "a reserved target slot keeps its seat");
    match &slots[at].1 {
        Slot::Reserved { transaction, .. } => assert!(
            *transaction == id,
            "the target slot is the reservation this transaction made"
        ),
        _ => panic!("a reserved target slot stays Reserved"),
    }
    frame.custody().leave();
    slots[at].1 = Slot::Resident(frame);
}

/// 回退源槽：`Moving → Resident`；帧从槽里取回，独占随之放开。
fn rollback_source(state: &mut FlatState, page: PageIndex, id: usize) {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("the source was a plain acquired source when it was claimed")
    };
    let (at, present) = slot::spot(slots, page);
    assert!(present, "a claimed source slot keeps its seat");
    let frame = match &slots[at].1 {
        Slot::Moving { frame, transaction } => {
            assert!(
                *transaction == id,
                "the source slot belongs to this transaction"
            );
            frame.clone()
        }
        _ => panic!("a claimed source slot stays Moving"),
    };
    frame.custody().leave();
    slots[at].1 = Slot::Resident(frame);
}

/// 回退目标槽：把空位还回它原本的样子（Fresh 就摘掉、`Missing` 就留墓碑）。
fn rollback_target(state: &mut FlatState, page: PageIndex, id: usize) {
    let FlatSource::Managed(slots) = &mut state.source else {
        unreachable!("the target was a plain acquired source when it was claimed")
    };
    let (at, present) = slot::spot(slots, page);
    assert!(present, "a reserved target slot keeps its seat");
    let was_missing = match slots[at].1 {
        Slot::Reserved {
            transaction,
            was_missing,
        } => {
            assert!(
                transaction == id,
                "the target slot is the reservation this transaction made"
            );
            was_missing
        }
        _ => panic!("a reserved target slot stays Reserved"),
    };
    if was_missing {
        slots[at].1 = Slot::Missing;
    } else {
        slots.remove(at);
    }
}
