//! 用户 TLS 地基 — 每线程独立 tp 指向的 TLS 块。

use env::MemoryResult;

use core::sync::atomic::{AtomicUsize, Ordering};

use env::{PAGE_SIZE, TaskId, TeamId, ViewId, ViewSelection};
use spinning_top::Spinlock;

#[path = "tls_slot.rs"]
mod slot;
use slot::Slot;

// 硬不变量：base() 仅在装配点后有效；bootstrap 仅在主线程出生点调用恰好一次。

pub const TLS_SIZE: usize = PAGE_SIZE;

const SLOTS: usize = 64;
static BLOCKS: Spinlock<[Slot; SLOTS]> = Spinlock::new([Slot::Free; SLOTS]);
static OWED: AtomicUsize = AtomicUsize::new(0);
static PENDING: AtomicUsize = AtomicUsize::new(0);

fn view_of(raw: usize) -> ViewId {
    ViewId::from_bytes(&raw.to_le_bytes()).expect("recorded view id")
}

fn reference_of(raw: usize) -> env::FlatReference {
    env::FlatReference::from_bytes(&raw.to_le_bytes()).expect("recorded reference id")
}

fn claim() -> Option<usize> {
    let owner = crate::memory::current_task();
    let mut slots = BLOCKS.lock();
    for (index, slot) in slots.iter_mut().enumerate() {
        if matches!(slot, Slot::Free) {
            *slot = Slot::Plotting { owner };
            return Some(index);
        }
    }
    None
}

fn abandon(index: usize) {
    BLOCKS.lock()[index] = Slot::Free;
}

fn commit(index: usize, address: usize, view: usize, pending: Option<(TaskId, usize)>) {
    let owner = crate::memory::current_task();
    BLOCKS.lock()[index] = Slot::Live {
        owner,
        address,
        view,
        pending,
    };
}

/// 两个责任都结了才腾格；`Closing`（在飞）永远不腾。
fn finish_if_done(index: usize) {
    let mut slots = BLOCKS.lock();
    if matches!(slots[index], Slot::Done { pending: None, .. }) {
        slots[index] = Slot::Free;
    }
}

/// 解一枚还没解掉的引用——**只在它的所有者那张表里**；失败/不是主人都保留。
fn release_pending(index: usize, pending: Option<(TaskId, usize)>) -> bool {
    let Some((owner, raw)) = pending else {
        return true;
    };
    if crate::memory::current_task() != owner {
        return false;
    }
    let current = {
        let slots = BLOCKS.lock();
        match slots[index] {
            Slot::Live { pending, .. }
            | Slot::Owed { pending, .. }
            | Slot::Closing { pending, .. }
            | Slot::Done { pending, .. } => pending,
            Slot::Free | Slot::Plotting { .. } => None,
        }
    };
    if current != pending {
        return false;
    }
    if crate::memory::release(reference_of(raw)).is_err() {
        return false;
    }
    let mut slots = BLOCKS.lock();
    match &mut slots[index] {
        Slot::Live { pending: held, .. }
        | Slot::Owed { pending: held, .. }
        | Slot::Closing { pending: held, .. }
        | Slot::Done { pending: held, .. } => {
            if *held != pending {
                return false;
            }
            *held = None;
        }
        Slot::Free | Slot::Plotting { .. } => return true,
    }
    drop(slots);
    PENDING.fetch_sub(1, Ordering::AcqRel);
    finish_if_done(index);
    true
}

fn claim_owed(index: usize) -> Option<(usize, Option<(TaskId, usize)>, usize)> {
    let current = crate::memory::current_task();
    let mut slots = BLOCKS.lock();
    match slots[index] {
        Slot::Owed {
            owner,
            address,
            view,
            pending,
        } if owner == current => {
            slots[index] = Slot::Closing { owner, pending };
            Some((view, pending, address))
        }
        _ => None,
    }
}

/// 按地址主张映射责任（归还入口）。
fn claim_close(address: usize) -> Option<(usize, usize, Option<(TaskId, usize)>)> {
    let current = crate::memory::current_task();
    let mut slots = BLOCKS.lock();
    for (index, slot) in slots.iter_mut().enumerate() {
        match *slot {
            Slot::Live {
                owner,
                address: held,
                view,
                pending,
            }
            | Slot::Owed {
                owner,
                address: held,
                view,
                pending,
            } if held == address && owner == current => {
                if matches!(slot, Slot::Live { .. }) {
                    OWED.fetch_add(1, Ordering::AcqRel);
                }
                *slot = Slot::Closing { owner, pending };
                return Some((index, view, pending));
            }
            _ => {}
        }
    }
    None
}

/// 映射已结清 / 没拆成（挂欠，扫描下次再试）。
fn mark_done(index: usize) {
    let mut slots = BLOCKS.lock();
    if let Slot::Closing { owner, pending } = slots[index] {
        slots[index] = Slot::Done { owner, pending };
        OWED.fetch_sub(1, Ordering::AcqRel);
    }
}

fn mark_owed(index: usize, address: usize, view: usize) {
    let mut slots = BLOCKS.lock();
    if let Slot::Closing { owner, pending } = slots[index] {
        slots[index] = Slot::Owed {
            owner,
            address,
            view,
            pending,
        };
    }
}

/// 把映射责任交给一次 `detach`：主张→拆→（结清 | 挂欠）。核心调用在锁外。
fn settle_mapping(index: usize, view: usize, pending: Option<(TaskId, usize)>, address: usize) {
    if crate::memory::detach(TeamId::new(0), view_of(view), ViewSelection::Whole).is_ok() {
        mark_done(index);
    } else {
        mark_owed(index, address, view);
    }
    release_pending(index, pending);
    finish_if_done(index);
}

/// 顺手扫一遍：欠着的映射、以及还没解掉的引用各试一次。
fn sweep() {
    if OWED.load(Ordering::Acquire) == 0 && PENDING.load(Ordering::Acquire) == 0 {
        return;
    }
    let current = crate::memory::current_task();
    for index in 0..SLOTS {
        let owner = BLOCKS.lock()[index].owner();
        if let Some(owner) = owner
            && owner != current
        {
            super::task::finished(owner);
            continue;
        }
        let (pending, done, owed) = {
            let slots = BLOCKS.lock();
            match slots[index] {
                Slot::Live { pending, .. } => (pending, false, false),
                Slot::Done { pending, .. } => (pending, true, false),
                Slot::Owed { .. } => (None, false, true),
                _ => (None, false, false),
            }
        };
        if owed {
            if let Some((view, pending, address)) = claim_owed(index) {
                settle_mapping(index, view, pending, address);
            }
            continue;
        }
        release_pending(index, pending);
        if done {
            finish_if_done(index);
        }
    }
}

pub(super) fn reap_task(task: TaskId) {
    let mut slots = BLOCKS.lock();
    for slot in slots.iter_mut() {
        if slot.owner() != Some(task) {
            continue;
        }
        if matches!(slot, Slot::Owed { .. } | Slot::Closing { .. }) {
            OWED.fetch_sub(1, Ordering::AcqRel);
        }
        if matches!(
            slot,
            Slot::Live {
                pending: Some(_),
                ..
            } | Slot::Owed {
                pending: Some(_),
                ..
            } | Slot::Closing {
                pending: Some(_),
                ..
            } | Slot::Done {
                pending: Some(_),
                ..
            }
        ) {
            PENDING.fetch_sub(1, Ordering::AcqRel);
        }
        *slot = Slot::Free;
    }
}

#[inline]
pub fn base() -> usize {
    let tp: usize;
    // SAFETY: 纯读 tp，无副作用。
    unsafe {
        core::arch::asm!("mv {0}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
    }
    tp
}

/// 本线程的 TLS 块（一整页）——错类型是 Memory 域的词汇（`OoM` / `NoRegion`）。
///
/// 私有（`Caller`）控制：这一段只归发起它的那个线程。
pub fn allocate() -> MemoryResult<usize> {
    sweep();
    // 表满就不取映射：不会出现“映射成功却没有槽记身份”。
    let Some(index) = claim() else {
        return Err(env::make_fail(env::MemoryFail::OoM));
    };
    let placed = crate::memory::allocate_raw(TLS_SIZE);
    let (address, view, reference) = match placed {
        Ok(placed) => placed,
        Err(error) => {
            abandon(index);
            return Err(error);
        }
    };
    // 关系已落地；这份引用现在要解掉——解不掉就记进这一格（带所有者）重试。
    let pending = match crate::memory::release(reference) {
        Ok(()) => None,
        Err(_) => {
            PENDING.fetch_add(1, Ordering::AcqRel);
            Some((crate::memory::current_task(), reference.get()))
        }
    };
    commit(index, address, view.get(), pending);
    Ok(address)
}

/// 归还本线程的 TLS 块（**任务退场前必调**）。
///
/// 为什么必须有：TLS 块是内核给的**一整页**，内核**不认**它是谁的——`bury` 只归还
/// `TaskIdent` 上记着的那两个 Span（栈 / trap 帧）。于是每个 spawn 出来的任务都把
/// 一页永久留在域空间里：churn 实测就是这条。
///
/// 归还按**准确的关系身份**执行（登记时记下的那个 `ViewId`），不是按地址猜所有权；
/// 同步未完成的失败保留身份，下一次再试，不当成功丢掉句柄。
pub fn deallocate() {
    let tp = base();
    if tp == 0 {
        return; // 未装配（理论上不可达）：不制造第二处失败
    }
    let Some((index, raw, pending)) = claim_close(tp) else {
        // 不是本线程的活块：可能是上次没结清的归还，扫一遍。
        sweep();
        return;
    };
    settle_mapping(index, raw, pending, tp);
}

/// # Safety
/// 仅在主线程出生点（`_start` → `main` 之间）调用恰好一次。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bootstrap() {
    let addr = allocate().expect("tls bootstrap alloc failed");
    // SAFETY: 写 tp（用户态自由；本线程刚出生，无旧值）。
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) addr, options(nomem, nostack, preserves_flags));
    }
}
