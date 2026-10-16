//! 无映射移交：稀疏页槽状态、迁移资格与两端移交。
//!
//! 判据都是相对读数：本函数进入前后的帧占用与逐类净额必须为零。
//! 这里不碰硬件：真实 Platform / DMA 场景本阶段未验。

#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr, VirtAddr};
use crate::core::memory::flat::allocator::statistics;
use crate::core::memory::flat::allocator::statistics::Kind;
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatError, FlatRange};
use crate::core::memory::nest::identity::fail_next_shootdown;
use crate::core::memory::nest::{Access, Nest, NestError, View, ViewRange};
use crate::core::memory::transfer::{Transfer, TransferError};
use crate::core::work::unit::space::SpaceBuilder;
use crate::machine::sync::SpinLock;

use super::support;

const BASE: usize = 0x6800_0000;

/// 在途探针要用的上下文：正在发布的那段关系与它的地址空间。
static PUBLISHED: SpinLock<Option<(Arc<Nest>, Arc<View>, ViewRange)>> = SpinLock::new(None);

fn one() -> PageCount {
    PageCount::new(1).expect("transfer: one page")
}

fn page(offset: usize) -> PageOffset {
    PageOffset::new(offset)
}

fn acquire(pages: usize) -> Arc<Flat> {
    Flat::acquire(Allocation::new(pages, PAGE_SIZE).expect("transfer: allocation"))
        .expect("transfer: acquire")
}

/// 从 `from` 窗口第 `from_page` 页移交给 `to` 窗口第 `to_page` 个槽。
fn transfer(
    from: &FlatRange,
    from_page: usize,
    to: &FlatRange,
    to_page: usize,
) -> Result<(), TransferError> {
    Transfer::new(from.clone(), page(from_page), to.clone(), page(to_page))?.execute()
}

fn write_byte(address: PhysAddr, value: u8) {
    unsafe { core::ptr::write_volatile(address.as_usize() as *mut u8, value) };
}

fn read_byte(address: PhysAddr) -> u8 {
    unsafe { core::ptr::read_volatile(address.as_usize() as *const u8) }
}

/// 给整页写一个位置相关的花纹，用来逐字节比对搬运前后。
fn fill_pattern(address: PhysAddr) {
    let bytes = address.as_usize() as *mut u8;
    for offset in 0..PAGE_SIZE {
        unsafe { core::ptr::write_volatile(bytes.add(offset), pattern_at(offset)) };
    }
}

/// 整页 4096 字节是否与花纹逐字节相同。
fn pattern_holds(address: PhysAddr) -> bool {
    let bytes = address.as_usize() as *const u8;
    (0..PAGE_SIZE)
        .all(|offset| unsafe { core::ptr::read_volatile(bytes.add(offset)) } == pattern_at(offset))
}

fn pattern_at(offset: usize) -> u8 {
    (offset as u8).wrapping_mul(31).wrapping_add(7)
}

/// 窗口内已放下承载的槽快照：`(相对偏移, 物理地址)`。`Missing` 是墓碑，不是承载。
fn placed(range: &FlatRange) -> Vec<(usize, usize)> {
    range
        .materialized_frames()
        .expect("transfer: slot snapshot")
        .into_iter()
        .map(|(offset, frame)| (offset.get(), frame.physical_address()))
        .collect()
}

/// 无映射移交的验收。
pub fn accept() {
    // 热身：首次建拆一个空间会触发一次性的惰性分配（核身份表与池的首次取自帧池），
    // 那不是本用例的账；先走一遍，再取基线。
    drop(
        SpaceBuilder::user()
            .build()
            .expect("transfer: warm-up space"),
    );

    let frames_before = statistics::frame_occupied() - statistics::block_occupied();
    let kinds_before = statistics::kinds();
    let cases: [(&str, fn()); 13] = [
        ("within one source", moves_within_one_source),
        ("round trip", round_trip_keeps_the_carrier),
        ("two sources", moves_between_two_sources),
        ("busy seats", rejects_non_empty_seats),
        ("cross same index", cross_source_same_index_is_allowed),
        ("imported carrier", imported_carriers_are_not_transferable),
        ("residency", residency_blocks_both_ends),
        (
            "mapped page",
            a_mapped_page_is_refused_until_it_is_withdrawn,
        ),
        ("oom rollback", out_of_memory_rolls_back),
        (
            "mid-window tombstone",
            a_mid_window_tombstone_is_not_a_complete_window,
        ),
        (
            "in-flight refusals",
            an_in_flight_claim_blocks_every_other_use,
        ),
        (
            "tombstone rollback",
            a_refused_move_keeps_a_tombstone_target,
        ),
        (
            "unreachable precedence",
            an_unreachable_owner_dominates_mapped_candidates,
        ),
    ];
    let mut off = Vec::new();
    for (name, run) in cases {
        let base = statistics::frame_occupied() - statistics::block_occupied();
        run();
        let now = statistics::frame_occupied() - statistics::block_occupied();
        if now != base {
            off.push((name, base, now));
        }
    }
    crate::expect!(off.is_empty(), "transfer: per-case net: {:?}", off);

    let frames_after = statistics::frame_occupied() - statistics::block_occupied();
    crate::expect!(
        frames_after == frames_before,
        "transfer: frame occupancy {frames_before} → {frames_after} (net must be 0)"
    );
    let kinds_after = statistics::kinds();
    for (kind, count) in kinds_after.nonzero() {
        if kind == Kind::Prime {
            // 块池取自帧池的整页；它们的账在 `block_occupied` 上，不在帧类目里。
            continue;
        }
        crate::expect!(
            count == kinds_before.get(kind),
            "transfer: kind {}: {} → {count} (net must be 0)",
            kind.name(),
            kinds_before.get(kind)
        );
    }
}

/// 同一份来源里换槽：承载、PA 与内容都不动；源成了墓碑，目标拿到同一个承载。
fn moves_within_one_source() {
    let flat = acquire(4);
    let whole = flat.whole();
    let source = whole.range(page(1), one()).expect("transfer: source page");
    let seat = whole.range(page(3), one()).expect("transfer: seat page");
    let pages = source.materialize().expect("transfer: materialize source");
    let address = pages
        .physical(PageOffset::ZERO)
        .expect("transfer: source pa");
    let carrier = Arc::downgrade(&source.materialized_frames().expect("transfer: slots")[0].1);
    write_byte(address, 0x5a);
    drop(pages);

    transfer(&source, 0, &seat, 0).expect("transfer: within one source");

    // 源是墓碑：不补零、不新分配，也不是“越界”那一类（`Missing` 走自己的身份）。
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Missing)),
        "transfer: a tombstone must not be re-materialized"
    );
    // 目标拿到同一个承载与同一个 PA，内容也没动。
    let moved = seat.materialized_frames().expect("transfer: seat slots");
    crate::expect!(moved.len() == 1, "transfer: the seat has exactly one slot");
    crate::expect!(
        Arc::ptr_eq(
            &moved[0].1,
            &carrier.upgrade().expect("transfer: carrier alive")
        ),
        "transfer: the very same frame lands in the seat"
    );
    crate::expect!(
        placed(&seat) == alloc::vec![(0, address.as_usize())],
        "transfer: the carrier keeps its physical address"
    );
    crate::expect!(read_byte(address) == 0x5a, "transfer: content is untouched");
    // 一次移动不新分配数据帧：窗内还是只有一个承载，且已在第 3 页。
    let slots = whole.materialized_frames().expect("transfer: snapshot");
    crate::expect!(
        slots.len() == 1 && slots[0].0.get() == 3,
        "transfer: a move adds no data frame"
    );
    // 别的空页仍然是 Fresh：物化会拿到它自己的承载，不复用移走的那个。
    let other = whole.range(page(0), one()).expect("transfer: other page");
    let other_pages = other
        .materialize()
        .expect("transfer: materialize the other page");
    crate::expect!(
        other_pages
            .physical(PageOffset::ZERO)
            .expect("transfer: other pa")
            != address,
        "transfer: a fresh page gets its own carrier"
    );
}

/// 双向：同一承载迁过去再迁回来，PA 与内容全程不变。
fn round_trip_keeps_the_carrier() {
    let flat = acquire(3);
    let whole = flat.whole();
    let first = whole.range(page(1), one()).expect("transfer: first");
    let second = whole.range(page(2), one()).expect("transfer: second");
    let address = first
        .materialize()
        .expect("transfer: materialize")
        .physical(PageOffset::ZERO)
        .expect("transfer: pa");
    write_byte(address, 0x3c);

    transfer(&first, 0, &second, 0).expect("transfer: out");
    crate::expect!(
        placed(&second) == alloc::vec![(0, address.as_usize())],
        "transfer: out keeps the physical address"
    );
    transfer(&second, 0, &first, 0).expect("transfer: back");
    crate::expect!(
        placed(&first) == alloc::vec![(0, address.as_usize())],
        "transfer: back keeps the physical address"
    );
    crate::expect!(
        read_byte(address) == 0x3c,
        "transfer: round trip keeps content"
    );
    crate::expect!(
        matches!(second.materialize(), Err(FlatError::Missing)),
        "transfer: the seat is a tombstone after the move back"
    );
}

/// 两个来源之间：`source → target` 与 `target → source` 走一对锁的两条地址序分支。
fn moves_between_two_sources() {
    let left = acquire(2);
    let right = acquire(2);
    let left_page = left
        .range(PageIndex::ZERO, one())
        .expect("transfer: left page");
    let right_page = right
        .range(PageIndex::ZERO, one())
        .expect("transfer: right page");
    let address = left_page
        .materialize()
        .expect("transfer: materialize left")
        .physical(PageOffset::ZERO)
        .expect("transfer: left pa");
    let carrier = Arc::downgrade(
        &left_page
            .materialized_frames()
            .expect("transfer: left slots")[0]
            .1,
    );
    fill_pattern(address);

    transfer(&left_page, 0, &right_page, 0).expect("transfer: across");
    crate::expect!(
        matches!(left_page.materialize(), Err(FlatError::Missing)),
        "transfer: the source is a tombstone after crossing"
    );
    crate::expect!(
        Arc::ptr_eq(
            &right_page
                .materialized_frames()
                .expect("transfer: right slots")[0]
                .1,
            &carrier.upgrade().expect("transfer: carrier alive")
        ),
        "transfer: the same frame crosses to the other source"
    );
    crate::expect!(
        placed(&right_page) == alloc::vec![(0, address.as_usize())],
        "transfer: crossing keeps the physical address"
    );
    crate::expect!(
        pattern_holds(address),
        "transfer: crossing keeps all 4096 bytes"
    );

    // 反方向：一对锁的另一条地址序分支。
    transfer(&right_page, 0, &left_page, 0).expect("transfer: across back");
    crate::expect!(
        placed(&left_page) == alloc::vec![(0, address.as_usize())],
        "transfer: crossing back keeps the physical address"
    );
    crate::expect!(
        matches!(right_page.materialize(), Err(FlatError::Missing)),
        "transfer: the other end is a tombstone after crossing back"
    );
}

/// 空槽之外都拒：源无承载、目标已占用、同槽、越界。
fn rejects_non_empty_seats() {
    let flat = acquire(3);
    let whole = flat.whole();
    let zero = whole.range(page(0), one()).expect("transfer: zero");
    let one_seat = whole.range(page(1), one()).expect("transfer: one");
    let two_seat = whole.range(page(2), one()).expect("transfer: two");

    // 源是 Fresh：没有可交出的承载。
    crate::expect!(
        transfer(&zero, 0, &one_seat, 0) == Err(TransferError::SourceEmpty),
        "transfer: a fresh source has nothing to hand over"
    );
    drop(zero.materialize().expect("transfer: materialize zero"));
    // 同槽：同一份来源上的同一页。
    crate::expect!(
        transfer(&zero, 0, &zero, 0) == Err(TransferError::DestinationOccupied),
        "transfer: the same slot is not a move"
    );
    // 目标已占用。
    drop(one_seat.materialize().expect("transfer: materialize one"));
    crate::expect!(
        transfer(&zero, 0, &one_seat, 0) == Err(TransferError::DestinationOccupied),
        "transfer: an occupied seat must be refused"
    );
    // 越界：两端各自显式检查。
    crate::expect!(
        matches!(
            Transfer::new(zero.clone(), page(1), one_seat.clone(), page(0)),
            Err(TransferError::OutOfRange)
        ),
        "transfer: the source page must be inside its window"
    );
    crate::expect!(
        matches!(
            Transfer::new(zero.clone(), page(0), one_seat.clone(), page(4)),
            Err(TransferError::OutOfRange)
        ),
        "transfer: the target page must be inside its window"
    );
    // 移走之后，源那一页是墓碑：也不是可交出的承载。
    transfer(&zero, 0, &two_seat, 0).expect("transfer: move away");
    crate::expect!(
        transfer(&zero, 0, &two_seat, 0) == Err(TransferError::SourceEmpty),
        "transfer: a tombstone has nothing to hand over"
    );
}

/// 不同来源上的同一个页号是合法选择：只有同一份来源上的同一页才是同槽。
fn cross_source_same_index_is_allowed() {
    let left = acquire(1);
    let right = acquire(1);
    let left_page = left
        .range(PageIndex::ZERO, one())
        .expect("transfer: left page");
    let right_page = right
        .range(PageIndex::ZERO, one())
        .expect("transfer: right page");
    let address = left_page
        .materialize()
        .expect("transfer: materialize")
        .physical(PageOffset::ZERO)
        .expect("transfer: pa");
    transfer(&left_page, 0, &right_page, 0).expect("transfer: same index across sources");
    crate::expect!(
        placed(&right_page) == alloc::vec![(0, address.as_usize())],
        "transfer: the same page index across two sources is a legal move"
    );
}

/// 被别处登记过的承载、`from_frames` 来源与借用来源都拒。
fn imported_carriers_are_not_transferable() {
    let flat = acquire(2);
    let whole = flat.whole();
    let seat = acquire(2);
    let seat_page = seat.range(PageIndex::ZERO, one()).expect("transfer: seat");
    drop(whole.materialize().expect("transfer: materialize"));
    let carrier = whole.materialized_frames().expect("transfer: slots")[0]
        .1
        .clone();

    // A 来自 acquire；它的承载后来被导入 C：从 A 迁出也必须拒。
    let holder = Flat::from_frames(
        PageCount::new(1).expect("one"),
        &[(PageIndex::ZERO, carrier.clone())],
    )
    .expect("transfer: import carrier");
    crate::expect!(
        transfer(&whole, 0, &seat_page, 0) == Err(TransferError::NotTransferable),
        "transfer: a carrier registered elsewhere must not move out"
    );
    // 幂等：同一份帧再导入一次仍然合法。
    drop(
        Flat::from_frames(
            PageCount::new(1).expect("one"),
            &[(PageIndex::ZERO, carrier)],
        )
        .expect("transfer: re-import is idempotent"),
    );
    crate::expect!(
        holder.page_count().get() == 1,
        "transfer: the importing source keeps its own shape"
    );

    // `from_frames` 建立的来源整段不参与迁移，包括之后才物化的帧。
    let imported =
        Flat::from_frames(PageCount::new(2).expect("two"), &[]).expect("transfer: empty import");
    let imported_range = imported.whole();
    let fresh = imported_range
        .range(page(0), one())
        .expect("transfer: imported page");
    drop(fresh.materialize().expect("transfer: materialize imported"));
    crate::expect!(
        transfer(&fresh, 0, &seat_page, 0) == Err(TransferError::NotTransferable),
        "transfer: an imported source never participates"
    );

    // 借用来源两端都拒。
    let borrowed = Flat::from_region(
        unsafe { BorrowedRegion::new(PhysAddr::from_raw(0x4000_0000), PAGE_SIZE) }
            .expect("transfer: borrow region"),
    )
    .expect("transfer: borrowed source");
    let borrowed_range = borrowed.whole();
    crate::expect!(
        transfer(&borrowed_range, 0, &seat_page, 0) == Err(TransferError::NotTransferable),
        "transfer: a borrowed source is refused"
    );
    crate::expect!(
        transfer(&whole, 0, &borrowed_range, 0) == Err(TransferError::NotTransferable),
        "transfer: a borrowed destination is refused"
    );
}

/// 两端只要有驻留覆盖就当场 Busy——排除是当场核的，不是调用方的承诺。
fn residency_blocks_both_ends() {
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    let seat = whole.range(page(1), one()).expect("transfer: seat");
    drop(source.materialize().expect("transfer: materialize"));

    let lease = whole.reside().expect("transfer: lease");
    crate::expect!(
        transfer(&source, 0, &seat, 0) == Err(TransferError::Busy),
        "transfer: a pinned source is busy"
    );
    drop(lease);
    let lease = seat.reside().expect("transfer: seat lease");
    crate::expect!(
        transfer(&source, 0, &seat, 0) == Err(TransferError::Busy),
        "transfer: a pinned destination is busy"
    );
    drop(lease);
    // 重叠驻留：同时盖住源与目标也一样当场 Busy；放掉之后不留残留。
    let overlap = whole.reside().expect("transfer: overlapping lease");
    crate::expect!(
        transfer(&source, 0, &seat, 0) == Err(TransferError::Busy),
        "transfer: an overlapping lease blocks both ends"
    );
    drop(overlap);
    crate::expect!(
        !whole.is_pinned(),
        "transfer: a released lease leaves no pin"
    );
    transfer(&source, 0, &seat, 0).expect("transfer: a released lease does not block");
    crate::expect!(
        !placed(&seat).is_empty(),
        "transfer: the move went through once the lease was gone"
    );
    crate::expect!(!whole.is_pinned(), "transfer: a move leaves no pin behind");
}

/// 中段墓碑：整窗不再是“完整物化”，补齐也被拒（不清零、不新分配）。
fn a_mid_window_tombstone_is_not_a_complete_window() {
    let flat = acquire(3);
    let whole = flat.whole();
    for index in 0..3 {
        let part = whole.range(page(index), one()).expect("transfer: part");
        drop(part.materialize().expect("transfer: materialize part"));
    }
    crate::expect!(
        whole
            .fully_materialized()
            .expect("transfer: complete window"),
        "transfer: three residents are a complete window"
    );

    // 把中间那一页搬到另一个来源：源窗口的中段留下墓碑。
    let seat = acquire(1);
    let seat_page = seat.whole();
    let middle = whole.range(page(1), one()).expect("transfer: middle");
    transfer(&middle, 0, &seat_page, 0).expect("transfer: move the middle page away");

    crate::expect!(
        !whole
            .fully_materialized()
            .expect("transfer: window with a hole"),
        "transfer: a mid-window tombstone is not a complete window"
    );
    crate::expect!(
        matches!(whole.materialize(), Err(FlatError::Missing)),
        "transfer: filling the window is refused at the tombstone"
    );
    let offsets: Vec<usize> = placed(&whole)
        .into_iter()
        .map(|(offset, _)| offset)
        .collect();
    crate::expect!(
        offsets == alloc::vec![0, 2],
        "transfer: the tombstone carries nothing"
    );
    crate::expect!(
        placed(&seat_page).len() == 1,
        "transfer: the moved page landed in the other source"
    );
}

/// 压在这份来源上的关系握着它窗口里的页：那些页当场 Busy。
///
/// 撤下其中一页并同步成功之后，那一页才可走；关系仍然握着的其他页仍然拒绝；
/// 地址空间失联（关系比空间活得久）也永久拒绝。
fn a_mapped_page_is_refused_until_it_is_withdrawn() {
    let flat = acquire(4);
    let whole = flat.whole();
    drop(whole.materialize().expect("transfer: materialize"));
    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let reservation =
        support::reserve(&space, VirtAddr::wrap(BASE), 4 * PAGE_SIZE).expect("transfer: reserve");
    let view = space
        .nest
        .attach(reservation, whole.clone(), Access::all(), rw)
        .expect("transfer: attach");
    let seat = acquire(2);
    let seat_page = seat.range(PageIndex::ZERO, one()).expect("transfer: seat");
    let source = whole.range(page(0), one()).expect("transfer: source");
    let other = whole.range(page(1), one()).expect("transfer: other page");
    let address = placed(&source)[0].1;

    crate::expect!(
        transfer(&source, 0, &seat_page, 0) == Err(TransferError::Busy),
        "transfer: a mapped page is busy"
    );
    // 同步失败：旧 PTE 还没有确认撤下，仍然拒绝。
    let clean_before_withdraw = !space.nest.has_pending_invalidation();
    crate::expect!(
        clean_before_withdraw,
        "transfer: the mapping starts with settled invalidation"
    );
    fail_next_shootdown(space.nest.asid());
    let selection = view
        .range(page(0), one())
        .expect("transfer: view selection");
    crate::expect!(
        matches!(space.nest.detach(selection), Err(NestError::Shootdown)),
        "transfer: the injected shootdown failure must be reported"
    );
    let still_active = view
        .active()
        .expect("transfer: active ranges after withdrawal")
        .iter()
        .any(|range| range.start.as_usize() <= BASE && BASE <= range.end());
    crate::expect!(
        clean_before_withdraw && !still_active,
        "transfer: stale clean state plus the new active hole must not authorize a move"
    );
    crate::expect!(
        space.nest.has_pending_invalidation(),
        "transfer: the active hole still has unconfirmed invalidation"
    );
    crate::expect!(
        transfer(&source, 0, &seat_page, 0) == Err(TransferError::Busy),
        "transfer: an unsettled sync keeps the page busy"
    );
    space
        .nest
        .retry_invalidation()
        .expect("transfer: retry invalidation");
    // 这一页的 PTE 已撤、已同步：它不再是这份关系握着的页。
    transfer(&source, 0, &seat_page, 0).expect("transfer: a withdrawn page moves");
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address)],
        "transfer: the withdrawn page keeps its carrier"
    );
    // 关系仍然握着第 1..3 页：那些页仍然拒绝。
    let second_seat = acquire(1);
    let second_page = second_seat.whole();
    crate::expect!(
        transfer(&other, 0, &second_page, 0) == Err(TransferError::Busy),
        "transfer: a relation still holds its other pages"
    );
    drop(view);
    drop(space);

    // 关系比它认住的地址空间活得久：旧 TLB 无从核实，永久拒绝，不当“没有旧 TLB”。
    let orphan_flat = acquire(1);
    let orphan_page = orphan_flat.whole();
    drop(
        orphan_page
            .materialize()
            .expect("transfer: materialize orphan"),
    );
    let orphan_space = SpaceBuilder::user()
        .build()
        .expect("transfer: orphan space");
    let orphan_reservation = support::reserve(&orphan_space, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("transfer: orphan reserve");
    let orphan = orphan_space
        .nest
        .attach(orphan_reservation, orphan_page.clone(), Access::all(), rw)
        .expect("transfer: orphan attach");
    drop(orphan_space);
    let third_seat = acquire(1);
    let third_page = third_seat.whole();
    crate::expect!(
        transfer(&orphan_page, 0, &third_page, 0) == Err(TransferError::NotTransferable),
        "transfer: a relation whose space is gone is not transferable"
    );
    drop(orphan);
}

/// 永久失联的属主优先于可重试的活动映射，不随候选顺序改变失败身份。
fn an_unreachable_owner_dominates_mapped_candidates() {
    let source = acquire(1).whole();
    drop(source.materialize().expect("transfer: materialize source"));
    let target = acquire(1).whole();
    let rw = Access::READ | Access::WRITE;
    let live_space = SpaceBuilder::user().build().expect("transfer: live space");
    let live_reservation = support::reserve(&live_space, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("transfer: live reservation");
    let live = live_space
        .nest
        .attach(live_reservation, source.clone(), rw, rw)
        .expect("transfer: live relation");
    let lost_space = SpaceBuilder::user().build().expect("transfer: lost space");
    let lost_reservation = support::reserve(&lost_space, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("transfer: lost reservation");
    let lost = lost_space
        .nest
        .attach(lost_reservation, source.clone(), rw, rw)
        .expect("transfer: relation whose owner will disappear");
    drop(lost_space);

    for _ in 0..2 {
        crate::expect!(
            transfer(&source, 0, &target, 0) == Err(TransferError::NotTransferable),
            "transfer: an unreachable owner dominates every retryable candidate"
        );
    }
    crate::expect!(
        placed(&source).len() == 1 && placed(&target).is_empty(),
        "transfer: refusal restores both slots"
    );
    drop(lost);
    drop(live);
    drop(live_space);
}

/// 占槽之后、结算之前：别的驻留、物化（PA 出口）与发布都在同一个在途窗口里被拒。
///
/// 探针借真实在途窗口核对排除关系；裁决随后看到关系还握着这一页，
/// 于是这次移交自己也 Busy，两端原样还回去。
fn an_in_flight_claim_blocks_every_other_use() {
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    drop(source.materialize().expect("transfer: materialize source"));
    let address = placed(&source)[0].1;
    write_byte(PhysAddr::from_raw(address), 0x11);

    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let reservation =
        support::reserve(&space, VirtAddr::wrap(BASE), 2 * PAGE_SIZE).expect("transfer: reserve");
    let view = space
        .nest
        .attach(reservation, whole.clone(), Access::all(), rw)
        .expect("transfer: attach");
    let seat = acquire(1);
    let seat_page = seat.whole();

    *PUBLISHED.lock() = Some((space.nest.clone(), view.clone(), view.whole()));
    crate::core::memory::transfer::arm_in_flight(Some(in_flight_refusals));
    let outcome = transfer(&source, 0, &seat_page, 0);
    crate::core::memory::transfer::arm_in_flight(None);
    *PUBLISHED.lock() = None;

    crate::expect!(
        outcome == Err(TransferError::Busy),
        "transfer: a relation holding the page refuses the move"
    );
    // 回退原态：源还是同一承载、同一 PA、同一内容；目标还是空的；不留驻留。
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)],
        "transfer: a refused claim restores the source"
    );
    crate::expect!(
        read_byte(PhysAddr::from_raw(address)) == 0x11,
        "transfer: content survives a refused claim"
    );
    crate::expect!(
        placed(&seat_page).is_empty(),
        "transfer: a refused claim leaves the seat empty"
    );
    crate::expect!(
        !source.is_pinned(),
        "transfer: a refused claim leaves no residency behind"
    );
    drop(
        source
            .reside()
            .expect("transfer: no residue blocks a new lease"),
    );
    // 关系走了之后同一页可以移。
    drop(view);
    drop(space);
    transfer(&source, 0, &seat_page, 0)
        .expect("transfer: the move lands once the relation is gone");
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address)],
        "transfer: the carrier reached the seat"
    );
}

/// 在途窗口里的排除：新驻留、物化（PA 出口）与一次发布都当场 Busy。
fn in_flight_refusals(source: &FlatRange, _page: PageIndex) {
    crate::expect!(
        matches!(source.reside(), Err(FlatError::Busy)),
        "transfer: an in-flight page refuses a new residency"
    );
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Busy)),
        "transfer: the PA exit needs a materialized window; an in-flight page refuses it"
    );
    let published = PUBLISHED.lock();
    let (nest, _, selection) = published.as_ref().expect("transfer: publish context");
    crate::expect!(
        matches!(
            nest.materialize_active(selection.clone()),
            Err(NestError::Busy)
        ),
        "transfer: publishing a window with an in-flight page is refused"
    );
}

/// 回退按目标原本的样子还回去：墓碑不会被抹成 Fresh。
fn a_refused_move_keeps_a_tombstone_target() {
    let source_flat = acquire(1);
    let source = source_flat.whole();
    drop(source.materialize().expect("transfer: materialize source"));

    // 目标第 0 页先是墓碑：把它的承载先移到第 1 页去。
    let target_flat = acquire(2);
    let target = target_flat.whole();
    let tombstone = target.range(page(0), one()).expect("transfer: target page");
    drop(
        tombstone
            .materialize()
            .expect("transfer: materialize target"),
    );
    let seat = target.range(page(1), one()).expect("transfer: empty seat");
    transfer(&tombstone, 0, &seat, 0).expect("transfer: make a tombstone");
    crate::expect!(
        matches!(tombstone.materialize(), Err(FlatError::Missing)),
        "transfer: the target page is a tombstone"
    );

    // 这段来源上挂一段关系：这次移交会在占槽之后被裁决拒掉。
    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let reservation =
        support::reserve(&space, VirtAddr::wrap(BASE), PAGE_SIZE).expect("transfer: reserve");
    let view = space
        .nest
        .attach(reservation, source.clone(), Access::all(), rw)
        .expect("transfer: attach");
    crate::expect!(
        transfer(&source, 0, &tombstone, 0) == Err(TransferError::Busy),
        "transfer: a relation holding the page refuses the move"
    );
    // 墓碑没被抹成 Fresh：物化仍然拒，而不是补出一个新承载。
    crate::expect!(
        matches!(tombstone.materialize(), Err(FlatError::Missing)),
        "transfer: a refused move keeps the target tombstone"
    );
    crate::expect!(
        placed(&source).len() == 1,
        "transfer: a refused move keeps the source carrier"
    );
    drop(view);
    drop(space);
    // 关系走了之后真的移过去：目标从墓碑变成承载。
    transfer(&source, 0, &tombstone, 0).expect("transfer: move onto a tombstone");
    crate::expect!(
        placed(&tombstone).len() == 1,
        "transfer: a tombstone target accepts a carrier"
    );
}

/// 准备阶段失败：槽一格不动、不新增数据帧、也不留下在途独占。
fn out_of_memory_rolls_back() {
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    let seat = acquire(2);
    let seat_page = seat.whole().range(page(1), one()).expect("transfer: seat");
    let pages = source.materialize().expect("transfer: materialize");
    let address = pages.physical(PageOffset::ZERO).expect("transfer: pa");
    let carrier = Arc::downgrade(&source.materialized_frames().expect("transfer: slots")[0].1);
    write_byte(address, 0x77);
    drop(pages);

    crate::core::memory::flat::fail_transfer_prepare();
    crate::expect!(
        transfer(&source, 0, &seat_page, 0) == Err(TransferError::OutOfMemory),
        "transfer: the injected preparation failure must be reported"
    );
    // 回滚净零：源仍是同一承载、同一 PA、同一内容；目标仍空。
    let slots = source
        .materialized_frames()
        .expect("transfer: source slots");
    crate::expect!(
        slots.len() == 1
            && Arc::ptr_eq(
                &slots[0].1,
                &carrier.upgrade().expect("transfer: carrier alive")
            ),
        "transfer: a failed preparation must not touch the source slot"
    );
    crate::expect!(
        placed(&source) == alloc::vec![(0, address.as_usize())],
        "transfer: a failed preparation must not change the carrier"
    );
    crate::expect!(
        read_byte(address) == 0x77,
        "transfer: content survives a failure"
    );
    crate::expect!(
        placed(&seat_page).is_empty(),
        "transfer: a failed preparation must not claim the seat"
    );

    // 重试：这回真的移过去，且仍是同一个承载。
    transfer(&source, 0, &seat_page, 0).expect("transfer: retry");
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Missing)),
        "transfer: the source becomes a tombstone after the retry"
    );
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address.as_usize())],
        "transfer: the retry lands the same carrier"
    );
    crate::expect!(
        read_byte(address) == 0x77,
        "transfer: content survives the retry"
    );
}
