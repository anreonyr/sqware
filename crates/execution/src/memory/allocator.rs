//! 用户堆：Talc 管理对象；页来源只走新五操作。

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId, ViewId, ViewSelection};
use spinning_top::{RawSpinlock, Spinlock};
use talc::{TalcLock, source::GlobalAllocSource};

use crate::memory::{self, PAGE_SIZE};

#[path = "allocator/pages.rs"]
mod pages;
#[path = "allocator/region.rs"]
mod region;
#[path = "allocator/slot.rs"]
mod slot;

use pages::Pages;
use region::Region;
use slot::Slot;

const BLOCK_SIZE: usize = 4 * PAGE_SIZE;
/// 同时存活的堆区上限：槽表是**启动预留的静态内存**，登记不再分配，
/// 也就不会回到同一个 `GlobalAlloc`。
const SLOTS: usize = 256;

/// 同时以 `Closing`/`Owed` 占着的映射责任数。
static OWED: AtomicUsize = AtomicUsize::new(0);
/// 还没解掉的引用个数。
static PENDING: AtomicUsize = AtomicUsize::new(0);

static REGIONS: Spinlock<[Slot; SLOTS]> = Spinlock::new([Slot::Free; SLOTS]);

fn view_of(raw: usize) -> ViewId {
    ViewId::from_bytes(&raw.to_le_bytes()).expect("recorded view id")
}

fn reference_of(raw: usize) -> env::FlatReference {
    env::FlatReference::from_bytes(&raw.to_le_bytes()).expect("recorded reference id")
}

/// 先占一个空槽；表满就返回 `None`——此时调用方还没做任何映射。
fn claim() -> Option<usize> {
    let mut slots = REGIONS.lock();
    for (index, slot) in slots.iter_mut().enumerate() {
        if matches!(slot, Slot::Free) {
            *slot = Slot::Plotting;
            return Some(index);
        }
    }
    None
}

/// 放弃一个没取成的占位。
fn abandon(index: usize) {
    REGIONS.lock()[index] = Slot::Free;
}

/// 取到映射后把真实身份填进已占的槽。
fn commit(index: usize, region: Region) {
    REGIONS.lock()[index] = Slot::Live {
        address: region.address,
        span: region.span,
        view: region.view,
        pending: region.pending,
    };
}

/// 只认仍欠着的映射；扫描期间复用成 `Live` 的槽不能被旧扫描撤下。
fn claim_owed(index: usize) -> Option<(usize, Option<(TaskId, usize)>, usize, usize)> {
    let mut slots = REGIONS.lock();
    match slots[index] {
        Slot::Owed {
            view,
            pending,
            span,
            address,
        } => {
            slots[index] = Slot::Closing { pending };
            Some((view, pending, span, address))
        }
        _ => None,
    }
}

/// 按地址主张映射责任（归还入口）。
fn claim_close(address: usize) -> Option<(usize, usize, Option<(TaskId, usize)>, usize)> {
    let mut slots = REGIONS.lock();
    for (index, slot) in slots.iter_mut().enumerate() {
        match *slot {
            Slot::Live {
                address: held,
                span,
                view,
                pending,
            }
            | Slot::Owed {
                address: held,
                span,
                view,
                pending,
            } if held == address => {
                if matches!(slot, Slot::Live { .. }) {
                    OWED.fetch_add(1, Ordering::AcqRel);
                }
                *slot = Slot::Closing { pending };
                return Some((index, view, pending, span));
            }
            _ => {}
        }
    }
    None
}

/// 映射责任已结清：转成只剩引用责任的那一格。
fn mark_done(index: usize) {
    let mut slots = REGIONS.lock();
    if let Slot::Closing { pending } = slots[index] {
        slots[index] = Slot::Done { pending };
        OWED.fetch_sub(1, Ordering::AcqRel);
    }
}

/// 映射没拆成：转成“欠着但没人在拆”，扫描下次可以再试。
fn mark_owed(index: usize, address: usize, span: usize, view: usize) {
    let mut slots = REGIONS.lock();
    if let Slot::Closing { pending } = slots[index] {
        // `Closing` 与 `Owed` 同计入 `OWED`，转态不重复计数。
        slots[index] = Slot::Owed {
            address,
            span,
            view,
            pending,
        };
    }
}

/// 解一枚还没解掉的引用——**只在它的所有者那张表里**。
///
/// 引用是表作用域的：别的 Task 拿着这个号去 Release 只会动错表。
fn release_pending(index: usize, pending: Option<(TaskId, usize)>) -> bool {
    let Some((owner, raw)) = pending else {
        return true;
    };
    if memory::current_task() != owner {
        return false;
    }
    let current = {
        let slots = REGIONS.lock();
        match slots[index] {
            Slot::Live { pending, .. }
            | Slot::Owed { pending, .. }
            | Slot::Closing { pending }
            | Slot::Done { pending } => pending,
            Slot::Free | Slot::Plotting => None,
        }
    };
    if current != pending {
        return false;
    }
    if memory::release(reference_of(raw)).is_err() {
        return false;
    }
    let mut slots = REGIONS.lock();
    match &mut slots[index] {
        Slot::Live { pending: held, .. }
        | Slot::Closing { pending: held }
        | Slot::Done { pending: held }
        | Slot::Owed { pending: held, .. } => {
            if *held != pending {
                return false;
            }
            *held = None;
        }
        Slot::Free | Slot::Plotting => return true,
    }
    drop(slots);
    PENDING.fetch_sub(1, Ordering::AcqRel);
    finish_if_done(index);
    true
}

/// 两个责任都结了才腾格；`Closing`（在飞）永远不腾。
fn finish_if_done(index: usize) {
    let mut slots = REGIONS.lock();
    let done = matches!(slots[index], Slot::Done { pending: None });
    if done {
        slots[index] = Slot::Free;
    }
}

/// 把映射责任交给一次 `detach`：主张→拆→（结清 | 挂欠）。核心调用在锁外。
fn settle_mapping(
    index: usize,
    view: usize,
    pending: Option<(TaskId, usize)>,
    span: usize,
    address: usize,
) {
    if memory::detach(TeamId::new(0), view_of(view), ViewSelection::Whole).is_ok() {
        mark_done(index);
    } else {
        mark_owed(index, address, span, view);
    }
    release_pending(index, pending);
    finish_if_done(index);
}

/// 顺手扫一遍：欠着的映射、以及还没解掉的引用各试一次。
/// `Closing` 是别人正在拆的，不动；“正在执行”本身就是不能结算的条件。
fn sweep() {
    if OWED.load(Ordering::Acquire) == 0 && PENDING.load(Ordering::Acquire) == 0 {
        return;
    }
    for index in 0..SLOTS {
        let (pending, done, owed) = {
            let slots = REGIONS.lock();
            match slots[index] {
                Slot::Live { pending, .. } => (pending, false, false),
                Slot::Done { pending } => (pending, true, false),
                Slot::Owed { .. } => (None, false, true),
                _ => (None, false, false),
            }
        };
        if owed {
            if let Some((view, pending, span, address)) = claim_owed(index) {
                settle_mapping(index, view, pending, span, address);
            }
            continue;
        }
        release_pending(index, pending);
        if done {
            finish_if_done(index);
        }
    }
}

type Heap = TalcLock<RawSpinlock, GlobalAllocSource<Pages>>;

// SAFETY: 每个区域独立申请；Talc 保留原始地址和大小，整块归还。
unsafe impl GlobalAlloc for Pages {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > PAGE_SIZE {
            return null_mut();
        }
        let Some(size) = layout.size().max(1).checked_next_multiple_of(PAGE_SIZE) else {
            return null_mut();
        };
        // 先把上一次失败的归还结掉；再占一个**实际空槽**——表满就不取映射。
        sweep();
        let Some(index) = claim() else {
            return null_mut();
        };
        // 堆区是**跨 Task 共享**的：控制权显式交给目标 Space 成员，
        // 别的线程归还 A 取的区不再 Denied。
        let placed = memory::allocate_shared_raw(size);
        let Ok((address, view, reference)) = placed else {
            abandon(index);
            return null_mut();
        };
        // 关系已落地；这份引用现在要解掉——解不掉就记进这一格（带所有者）重试。
        let pending = match memory::release(reference) {
            Ok(()) => None,
            Err(_) => {
                PENDING.fetch_add(1, Ordering::AcqRel);
                Some((memory::current_task(), reference.get()))
            }
        };
        commit(
            index,
            Region {
                address,
                span: size,
                view: view.get(),
                pending,
            },
        );
        address as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let address = ptr as usize;
        let Some((index, view, pending, span)) = claim_close(address) else {
            // 不是本堆的活记录：可能是上次没结清的归还，扫一遍。
            sweep();
            return;
        };
        debug_assert_eq!(span, layout.size().max(1).next_multiple_of(PAGE_SIZE));
        settle_mapping(index, view, pending, span, address);
    }
}

// 页来源不使用用户堆，避免获取或归还区域时递归进入 Talc。
#[cfg(target_arch = "riscv64")]
#[global_allocator]
static HEAP: Heap = Heap::new(GlobalAllocSource::with_block_size(Pages, BLOCK_SIZE));

#[cfg(test)]
#[path = "allocator/tests.rs"]
mod tests;
