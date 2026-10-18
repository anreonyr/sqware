//! 移交：稀疏页槽状态、来源迁移资格、压着来源页的关系与两端接管。
//!
//! 判据都是相对读数：本函数进入前后的帧占用与逐类净额必须为零。
//! 这里不碰硬件：真实 Platform / DMA 场景本阶段未验。

#![cfg(debug_assertions)]

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr, VirtAddr};
use crate::core::memory::flat::allocator::statistics;
use crate::core::memory::flat::allocator::statistics::Kind;
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatError, FlatRange};
use crate::core::memory::nest::fail_unpublish_prepare;
use crate::core::memory::nest::identity::fail_next_shootdown;
use crate::core::memory::nest::source_views;
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, ActiveRange, Nest, NestError, View, ViewRange};
use crate::core::memory::transfer::{Transfer, TransferError};
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::machine::sync::SpinLock;

use super::support;

const BASE: usize = 0x6800_0000;
/// 左空间里两条关系的起点：页 0..2 与页 2..4。
const LEFT_A: usize = 0x6900_0000;
const LEFT_B: usize = 0x6940_0000;
/// 右空间里错位切片的起点：页 1..3。
const RIGHT_A: usize = 0x6980_0000;
/// 2 MiB 大叶的跨度（同时也是 `BASE` 那一段窗口的大小与对齐）。
const MEGA: usize = 1 << 21;

/// 在途探针要用的上下文：正在发布的那段关系与它的地址空间。
static PUBLISHED: SpinLock<Option<(Arc<Nest>, Arc<View>, ViewRange)>> = SpinLock::new(None);

/// 在途探针要用的另一份上下文：一次移交的两端窗口与各自页偏移。
static ENDS: SpinLock<Option<(FlatRange, PageOffset, FlatRange, PageOffset)>> = SpinLock::new(None);

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
    Transfer::new(from.clone(), page(from_page), to.clone(), page(to_page))?
        .execute()
        .result
}

/// 清单里有没有这一条关系（按 `Arc` 身份）。
fn names(affected: &[Weak<View>], view: &Arc<View>) -> bool {
    affected.iter().any(|candidate| {
        candidate
            .upgrade()
            .is_some_and(|known| Arc::ptr_eq(&known, view))
    })
}

pub(super) fn write_byte(address: PhysAddr, value: u8) {
    unsafe { core::ptr::write_volatile(address.as_usize() as *mut u8, value) };
}

pub(super) fn read_byte(address: PhysAddr) -> u8 {
    unsafe { core::ptr::read_volatile(address.as_usize() as *const u8) }
}

/// 给整页写一个位置相关的花纹，用来逐字节比对搬运前后。
pub(super) fn fill_pattern(address: PhysAddr) {
    let bytes = address.as_usize() as *mut u8;
    for offset in 0..PAGE_SIZE {
        unsafe { core::ptr::write_volatile(bytes.add(offset), pattern_at(offset)) };
    }
}

/// 整页 4096 字节是否与花纹逐字节相同。
pub(super) fn pattern_holds(address: PhysAddr) -> bool {
    let bytes = address.as_usize() as *const u8;
    (0..PAGE_SIZE)
        .all(|offset| unsafe { core::ptr::read_volatile(bytes.add(offset)) } == pattern_at(offset))
}

pub(super) fn pattern_at(offset: usize) -> u8 {
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

/// `range` 内从第 `from` 页起 `pages` 页的切片。
fn slice(range: &FlatRange, from: usize, pages: usize) -> FlatRange {
    range
        .range(
            page(from),
            PageCount::new(pages).expect("transfer: slice pages"),
        )
        .expect("transfer: slice")
}

/// 把一段来源切片挂到 `space` 的 `at` 上；当前权限就是 `access`。
fn map_slice(space: &Space, at: VirtAddr, source: FlatRange, access: Access) -> Arc<View> {
    let size = source
        .page_count()
        .checked_bytes()
        .expect("transfer: slice bytes");
    let reservation = support::reserve(space, at, size).expect("transfer: reserve the slice");
    space
        .nest
        .attach(reservation, source, Access::all(), access)
        .expect("transfer: attach the slice")
}

/// 自己的空间里、`BASE` 上的那一条关系。
fn relation(source: &FlatRange, access: Access) -> (Space, Arc<View>) {
    let space = SpaceBuilder::user()
        .build()
        .expect("transfer: relation space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), access);
    (space, view)
}

/// 一段关系的身份、窗口与活动形状：撤页不许动这三样。
fn shape(view: &Arc<View>) -> (usize, (VirtAddr, usize), Vec<(usize, usize, u64, u64)>) {
    let active = view.active().expect("transfer: active shape");
    (
        view.identity(),
        view.initial_range(),
        active
            .iter()
            .map(|range| {
                (
                    range.start.as_usize(),
                    range.size.get(),
                    range.ceiling.bits(),
                    range.current.bits(),
                )
            })
            .collect(),
    )
}

/// 移交的验收。
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
    let cases: [(&str, fn()); 23] = [
        ("within one source", moves_within_one_source),
        ("round trip", round_trip_keeps_the_carrier),
        ("two sources", moves_between_two_sources),
        ("busy seats", rejects_non_empty_seats),
        ("cross same index", cross_source_same_index_is_allowed),
        ("imported carrier", imported_carriers_are_not_transferable),
        ("residency", residency_blocks_both_ends),
        ("mapped page", a_mapped_page_moves_and_gives_up_its_pte),
        ("oom rollback", out_of_memory_rolls_back),
        (
            "mid-window tombstone",
            a_mid_window_tombstone_is_not_a_complete_window,
        ),
        ("in-flight refusals", an_in_flight_claim_refuses_other_uses),
        ("nest sync failure", a_failed_shootdown_restores_both_ends),
        (
            "unpublish preparation",
            a_failed_unpublish_prepare_keeps_the_mapping,
        ),
        (
            "later nest preparation",
            a_failed_unpublish_prepare_stops_the_line,
        ),
        ("tombstone rollback", a_failed_move_keeps_a_tombstone_target),
        (
            "unreachable precedence",
            an_unreachable_owner_dominates_mapped_candidates,
        ),
        ("global entry", a_global_mapping_is_not_transferable),
        (
            "former global",
            a_withdrawn_global_relation_keeps_the_source_untransferable,
        ),
        (
            "candidate preparation",
            a_failed_candidate_preparation_changes_nothing,
        ),
        (
            "failed move names both ends",
            a_failed_move_still_names_both_ends,
        ),
        (
            "both ends one relation",
            a_relation_over_both_ends_is_named_once,
        ),
        (
            "materializability",
            can_materialize_reports_page_availability,
        ),
        (
            "plain acquire window",
            a_plain_acquire_window_hands_over_one_page,
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
    // 资格先于页槽状态：这一页连承载都还没有，也不是可迁的来源。
    crate::expect!(
        transfer(&fresh, 0, &seat_page, 0) == Err(TransferError::NotTransferable),
        "transfer: an imported source never participates, even before it is materialized"
    );
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

/// 有映射的一页：逐 Nest 撤页并确认之后才移交。
///
/// 两个地址空间、三条关系（其中一条已经撤出一段，留成活动洞）；被测页同时被
/// 两个 Nest 压着。撤页只拿掉这一页的 PTE 与发布记录：关系的身份、窗口、活动形状
/// 与其余页的权限一律不动。
fn a_mapped_page_moves_and_gives_up_its_pte() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(4);
    let whole = flat.whole();
    for index in 0..4 {
        let page = whole.range(page(index), one()).expect("transfer: page");
        let pages = page.materialize().expect("transfer: materialize page");
        fill_pattern(pages.physical(PageOffset::ZERO).expect("transfer: pa"));
        drop(pages);
    }
    let carriers = whole.materialized_frames().expect("transfer: slots");
    let carrier_one = Arc::downgrade(&carriers[1].1);
    let carrier_two = Arc::downgrade(&carriers[2].1);
    let address_one = carriers[1].1.physical_address();
    let address_two = carriers[2].1.physical_address();
    drop(carriers);

    let left = SpaceBuilder::user().build().expect("transfer: left space");
    let right = SpaceBuilder::user().build().expect("transfer: right space");
    let alpha = map_slice(&left, VirtAddr::wrap(LEFT_A), slice(&whole, 0, 2), rw);
    let beta = map_slice(&left, VirtAddr::wrap(LEFT_B), slice(&whole, 2, 2), rw);
    // 右空间的关系是左空间那两条的错位切片：同一份来源上多 View、多 Nest。
    let gamma = map_slice(&right, VirtAddr::wrap(RIGHT_A), slice(&whole, 1, 2), rw);
    // 左空间的第一条关系先撤下一段：那个虚拟页成为活动洞。
    left.nest
        .detach(
            alpha
                .range(page(1), one())
                .expect("transfer: hole selection"),
        )
        .expect("transfer: withdraw the middle page");

    let alpha_shape = shape(&alpha);
    let beta_shape = shape(&beta);
    let gamma_shape = shape(&gamma);
    let left_keep = left
        .nest
        .translate(VirtAddr::wrap(LEFT_A))
        .expect("transfer: left page 0")
        .1
        .bits();
    let right_keep = right
        .nest
        .translate(VirtAddr::wrap(RIGHT_A))
        .expect("transfer: right page 0")
        .1
        .bits();
    crate::expect!(
        left.nest
            .translate(VirtAddr::wrap(LEFT_A + PAGE_SIZE))
            .is_none()
            && left.nest.translate(VirtAddr::wrap(LEFT_B)).is_some()
            && right
                .nest
                .translate(VirtAddr::wrap(RIGHT_A + PAGE_SIZE))
                .is_some(),
        "transfer: the starting state is one hole plus three live pages"
    );

    let seat = acquire(1);
    let seat_page = seat.whole();
    transfer(&slice(&whole, 2, 1), 0, &seat_page, 0).expect("transfer: a mapped page moves");

    // 同一个承载、同一个 PA、整页内容；目标的发布是惰性的（这里不要求它已经有 PTE）。
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address_two)],
        "transfer: the moved page lands with the same carrier"
    );
    crate::expect!(
        Arc::ptr_eq(
            &seat_page
                .materialized_frames()
                .expect("transfer: seat slots")[0]
                .1,
            &carrier_two.upgrade().expect("transfer: carrier alive")
        ),
        "transfer: the very same frame lands in the seat"
    );
    crate::expect!(
        pattern_holds(PhysAddr::from_raw(address_two)),
        "transfer: the move keeps all 4096 bytes"
    );
    crate::expect!(
        matches!(slice(&whole, 2, 1).materialize(), Err(FlatError::Missing)),
        "transfer: the source page becomes a tombstone"
    );
    // 旧源 PTE 消失；关系还压在别的地方，活动洞也不回填。
    crate::expect!(
        left.nest.translate(VirtAddr::wrap(LEFT_B)).is_none()
            && right
                .nest
                .translate(VirtAddr::wrap(RIGHT_A + PAGE_SIZE))
                .is_none(),
        "transfer: every relation gives up the moved page"
    );
    crate::expect!(
        left.nest
            .translate(VirtAddr::wrap(LEFT_B + PAGE_SIZE))
            .is_some()
            && right.nest.translate(VirtAddr::wrap(RIGHT_A)).is_some(),
        "transfer: the other pages stay mapped"
    );
    crate::expect!(
        left.nest
            .translate(VirtAddr::wrap(LEFT_A + PAGE_SIZE))
            .is_none(),
        "transfer: an active hole is not refilled"
    );
    // 身份、窗口、活动形状与其余页的权限都不动。
    crate::expect!(
        shape(&alpha) == alpha_shape && shape(&beta) == beta_shape && shape(&gamma) == gamma_shape,
        "transfer: identities and active shapes survive the withdrawal"
    );
    crate::expect!(
        left.nest
            .translate(VirtAddr::wrap(LEFT_A))
            .expect("transfer: left page 0")
            .1
            .bits()
            == left_keep
            && right
                .nest
                .translate(VirtAddr::wrap(RIGHT_A))
                .expect("transfer: right page 0")
                .1
                .bits()
                == right_keep,
        "transfer: the surviving pages keep their permissions"
    );
    crate::expect!(
        Arc::ptr_eq(
            &left
                .nest
                .probe(VirtAddr::wrap(LEFT_B))
                .expect("transfer: the relation stays"),
            &beta
        ),
        "transfer: the relation keeps its index entry"
    );

    // 活动洞里的一页也在右空间上压着：两条关系一撤、一空，一起放过。
    let second = acquire(1);
    let second_page = second.whole();
    transfer(&slice(&whole, 1, 1), 0, &second_page, 0).expect("transfer: a holed page moves");
    crate::expect!(
        placed(&second_page) == alloc::vec![(0, address_one)],
        "transfer: the holed page lands with the same carrier"
    );
    crate::expect!(
        Arc::ptr_eq(
            &second_page.materialized_frames().expect("transfer: slots")[0].1,
            &carrier_one.upgrade().expect("transfer: carrier alive")
        ),
        "transfer: the very same frame lands in the second seat"
    );
    crate::expect!(
        right.nest.translate(VirtAddr::wrap(RIGHT_A)).is_none(),
        "transfer: the slice on the other nest gives up its PTE"
    );
    crate::expect!(
        shape(&alpha) == alpha_shape,
        "transfer: an active hole does not change the shape"
    );
    // 落定之后整个地址空间自检：索引、发布记录与硬件 PTE 一致。
    left.nest.audit();
    right.nest.audit();
    drop(gamma);
    drop(beta);
    drop(alpha);
    drop(right);
    drop(left);
}

/// 逐 Nest 注入同步失败：已经确认撤下的那一页不回填，两端按原态还原。
///
/// 失败安在后一个处理的 Nest 上（次序就是关联链的次序），于是前一个 Nest 的
/// 撤页与确认已经成功；这次失败只把两个页槽还原，源可以按需重新发布，
/// 重试把承载落到目标。
fn a_failed_shootdown_restores_both_ends() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(1);
    let source = flat.whole();
    let pages = source.materialize().expect("transfer: materialize");
    let carrier = pages.physical(PageOffset::ZERO).expect("transfer: pa");
    let address = carrier.as_usize();
    fill_pattern(carrier);
    drop(pages);

    let left = SpaceBuilder::user().build().expect("transfer: left space");
    let right = SpaceBuilder::user().build().expect("transfer: right space");
    let left_view = map_slice(&left, VirtAddr::wrap(BASE), source.clone(), rw);
    let right_view = map_slice(&right, VirtAddr::wrap(BASE), source.clone(), rw);
    let seat = acquire(1);
    let seat_page = seat.whole();

    let pressings =
        source_views::pressings_over(&source, PageIndex::ZERO).expect("transfer: pressings");
    crate::expect!(
        pressings.len() == 2,
        "transfer: two relations press the source page"
    );
    crate::expect!(
        left.nest.translate(VirtAddr::wrap(BASE)).is_some()
            && right.nest.translate(VirtAddr::wrap(BASE)).is_some(),
        "transfer: both relations start with a live mapping"
    );
    let first = pressings[0]
        .nest
        .as_ref()
        .expect("transfer: reachable owner");
    let later = pressings[1]
        .nest
        .as_ref()
        .expect("transfer: reachable owner");
    let first_va = pressings[0].address;
    let later_va = pressings[1].address;
    let first_view = pressings[0].view.clone();
    let later_view = pressings[1].view.clone();
    let first_shape = shape(&first_view);
    let later_shape = shape(&later_view);

    fail_next_shootdown(later.asid());
    crate::expect!(
        transfer(&source, 0, &seat_page, 0) == Err(TransferError::Busy),
        "transfer: an unconfirmed sync is a retryable Busy"
    );
    // 两端还原：源还是同一承载、同一 PA、同一内容；目标仍然空。
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)],
        "transfer: a failed per-nest sync restores the source slot"
    );
    crate::expect!(
        pattern_holds(carrier),
        "transfer: content survives the failure"
    );
    crate::expect!(
        placed(&seat_page).is_empty(),
        "transfer: a failed per-nest sync leaves the seat empty"
    );
    // 前一个 Nest 的撤页已经确认：不回填；后一个 Nest 撤下在前、同步在后，也已撤下。
    crate::expect!(
        first.translate(first_va).is_none(),
        "transfer: a confirmed removal is not rolled back"
    );
    crate::expect!(
        later.has_pending_invalidation(),
        "transfer: the failed sync keeps its responsibility"
    );
    crate::expect!(
        shape(&first_view) == first_shape && shape(&later_view) == later_shape,
        "transfer: a failed sync does not disturb the relations"
    );
    later
        .retry_invalidation()
        .expect("transfer: settle the failed sync");
    crate::expect!(
        later.translate(later_va).is_none(),
        "transfer: the withdrawal was real once the sync was settled"
    );
    // 源可重新发布：承载还在源槽里，按需把这一页发布回来。
    later
        .materialize_active(
            later_view
                .range(page(0), one())
                .expect("transfer: selection"),
        )
        .expect("transfer: republish after a failed sync");
    crate::expect!(
        later.translate(later_va).map(|(physical, _)| physical) == Some(carrier),
        "transfer: the republished page keeps its carrier"
    );
    first
        .materialize_active(
            first_view
                .range(page(0), one())
                .expect("transfer: selection"),
        )
        .expect("transfer: republish a confirmed removal");

    // 重试：这回没有注入，两个 Nest 都确认，承载落到目标。
    transfer(&source, 0, &seat_page, 0).expect("transfer: the retry lands the carrier");
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address)],
        "transfer: the retry lands the same carrier"
    );
    crate::expect!(
        pattern_holds(carrier),
        "transfer: the retry keeps the content"
    );
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Missing)),
        "transfer: the retried source becomes a tombstone"
    );
    crate::expect!(
        first.translate(first_va).is_none() && later.translate(later_va).is_none(),
        "transfer: the retry gives up both mapped PTEs"
    );
    first.audit();
    later.audit();
    drop(first_view);
    drop(later_view);
    drop(seat_page);
    drop(seat);
    drop(left_view);
    drop(right_view);
    drop(left);
    drop(right);
}

/// 撤页的准备阶段失败：一页 PTE 都不撤，两端按原态还原，也不起同步。
fn a_failed_unpublish_prepare_keeps_the_mapping() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(1);
    let source = flat.whole();
    let pages = source.materialize().expect("transfer: materialize");
    let carrier = pages.physical(PageOffset::ZERO).expect("transfer: pa");
    let address = carrier.as_usize();
    fill_pattern(carrier);
    drop(pages);
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    let seat = acquire(1);
    let seat_page = seat.whole();

    fail_unpublish_prepare(space.nest.asid());
    crate::expect!(
        transfer(&source, 0, &seat_page, 0) == Err(TransferError::OutOfMemory),
        "transfer: the injected per-page preparation failure is reported"
    );
    crate::expect!(
        space
            .nest
            .translate(VirtAddr::wrap(BASE))
            .map(|(physical, _)| physical)
            == Some(carrier),
        "transfer: a failed preparation keeps the mapped PTE"
    );
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)],
        "transfer: a failed preparation keeps the source carrier"
    );
    crate::expect!(
        placed(&seat_page).is_empty(),
        "transfer: a failed preparation leaves the seat empty"
    );
    crate::expect!(
        !space.nest.has_pending_invalidation(),
        "transfer: a failed preparation starts no sync"
    );
    space.nest.audit();
    drop(view);
    drop(space);
    // 重试：这回真的移过去，承载与内容不变。
    transfer(&source, 0, &seat_page, 0).expect("transfer: the retry lands the carrier");
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address)],
        "transfer: the retry lands the same carrier"
    );
    crate::expect!(
        pattern_holds(carrier),
        "transfer: the retry keeps the content"
    );
}

/// 逐 Nest 的准备失败：前面那个 Nest 的撤页已经确认，后面那个在原地失败。
///
/// 准备失败在任何 PTE 被撤下之前，所以它既不该回填前面的撤页，也不该起同步；
/// 两端按原态还原，目标墓碑不被抹成 Fresh，重试把承载落到墓碑上。
fn a_failed_unpublish_prepare_stops_the_line() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(1);
    let source = flat.whole();
    let pages = source.materialize().expect("transfer: materialize");
    let carrier = pages.physical(PageOffset::ZERO).expect("transfer: pa");
    let address = carrier.as_usize();
    fill_pattern(carrier);
    drop(pages);

    let left = SpaceBuilder::user().build().expect("transfer: left space");
    let right = SpaceBuilder::user().build().expect("transfer: right space");
    drop(map_slice(&left, VirtAddr::wrap(BASE), source.clone(), rw));
    drop(map_slice(&right, VirtAddr::wrap(BASE), source.clone(), rw));
    // 目标先是墓碑：回退要原样还回去。
    let target_flat = acquire(2);
    let target = target_flat.whole();
    let tombstone = target.range(page(0), one()).expect("transfer: target page");
    drop(
        tombstone
            .materialize()
            .expect("transfer: materialize target"),
    );
    let other = target.range(page(1), one()).expect("transfer: seat");
    transfer(&tombstone, 0, &other, 0).expect("transfer: make a tombstone");
    crate::expect!(
        matches!(tombstone.materialize(), Err(FlatError::Missing)),
        "transfer: the target page is a tombstone"
    );

    let pressings =
        source_views::pressings_over(&source, PageIndex::ZERO).expect("transfer: pressings");
    crate::expect!(
        pressings.len() == 2,
        "transfer: two relations press the source page"
    );
    let first = pressings[0]
        .nest
        .as_ref()
        .expect("transfer: reachable owner");
    let later = pressings[1]
        .nest
        .as_ref()
        .expect("transfer: reachable owner");
    let first_va = pressings[0].address;
    let later_va = pressings[1].address;

    fail_unpublish_prepare(later.asid());
    crate::expect!(
        transfer(&source, 0, &tombstone, 0) == Err(TransferError::OutOfMemory),
        "transfer: the later nest's preparation failure is reported"
    );
    // 前面那个 Nest 的撤页已经确认：不回填。
    crate::expect!(
        first.translate(first_va).is_none(),
        "transfer: a confirmed withdrawal is not rolled back"
    );
    // 后面那个 Nest 在原地失败：PTE 还在，也没起同步。
    crate::expect!(
        later.translate(later_va).map(|(physical, _)| physical) == Some(carrier),
        "transfer: the failing nest keeps its mapping"
    );
    crate::expect!(
        !later.has_pending_invalidation() && !first.has_pending_invalidation(),
        "transfer: a preparation failure starts no sync"
    );
    // 两端还原，墓碑不被抹成 Fresh。
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)],
        "transfer: a per-nest preparation failure restores the source slot"
    );
    crate::expect!(
        matches!(tombstone.materialize(), Err(FlatError::Missing)),
        "transfer: a per-nest preparation failure keeps the target tombstone"
    );
    crate::expect!(
        pattern_holds(carrier),
        "transfer: content survives the failure"
    );

    // 重试：这回没有注入，两个 Nest 都撤下并确认，承载落到墓碑上。
    transfer(&source, 0, &tombstone, 0).expect("transfer: the retry lands the carrier");
    crate::expect!(
        placed(&tombstone) == alloc::vec![(0, address)],
        "transfer: the retry lands the same carrier"
    );
    crate::expect!(
        pattern_holds(carrier),
        "transfer: the retry keeps the content"
    );
    crate::expect!(
        first.translate(first_va).is_none() && later.translate(later_va).is_none(),
        "transfer: the retry gives up both mapped PTEs"
    );
    first.audit();
    later.audit();
    drop(left);
    drop(right);
}

/// 永久失联的属主优先于可重试的活动映射，两种注册顺序都是同一条永久拒绝。
fn an_unreachable_owner_dominates_mapped_candidates() {
    let from_live_first = live_and_orphan_over_one_source(true);
    let from_lost_first = live_and_orphan_over_one_source(false);
    crate::expect!(
        from_live_first != from_lost_first,
        "transfer: the two registration orders really land on opposite candidate orders"
    );
}

/// 同一份来源上同时压着一条活关系与一条失联关系：这次移交必须永久拒绝，两端不变，
/// 而活关系那一页的 PTE 也不能先被撕掉再发现失联。
///
/// `live_first` 决定两条关系的注册先后；返回值说明这一次枚举把哪一条排在前面
/// （`true` = 失联那条在前），用来确认两种注册顺序真的各测了一次。
fn live_and_orphan_over_one_source(live_first: bool) -> bool {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(1);
    let source = flat.whole();
    drop(source.materialize().expect("transfer: materialize source"));
    let address = placed(&source)[0].1;
    let target = acquire(1).whole();

    let (first_space, first_view) = relation(&source, rw);
    let (second_space, second_view) = relation(&source, rw);
    // 把一条变成失联：它的空间先走，关系比空间活得久。
    let (live_space, live, lost) = if live_first {
        drop(second_space);
        (first_space, first_view, second_view)
    } else {
        drop(first_space);
        (second_space, second_view, first_view)
    };

    let pressings =
        source_views::pressings_over(&source, PageIndex::ZERO).expect("transfer: pressings");
    crate::expect!(
        pressings.len() == 2 && pressings.iter().any(|pressing| pressing.nest.is_none()),
        "transfer: one live and one orphan relation press the page"
    );
    let orphan_first = pressings[0].nest.is_none();

    crate::expect!(
        transfer(&source, 0, &target, 0) == Err(TransferError::NotTransferable),
        "transfer: an unreachable owner is a permanent refusal"
    );
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)],
        "transfer: a permanent refusal restores the source slot"
    );
    crate::expect!(
        placed(&target).is_empty(),
        "transfer: a permanent refusal leaves the seat empty"
    );
    // 活关系那一页也得还在：永久拒绝不许先把普通页撤了、再发现失联。
    crate::expect!(
        live_space
            .nest
            .translate(VirtAddr::wrap(BASE))
            .map(|(physical, _)| physical)
            == Some(PhysAddr::from_raw(address)),
        "transfer: the live page keeps its PTE through the permanent refusal"
    );
    live_space.nest.audit();
    drop(live);
    drop(live_space);
    drop(lost);
    orphan_first
}

/// 全局条目不是可迁移的来源：本关系自己的 ASID 定向失效覆盖不到它，
/// 于是“旧 TLB 已确认撤下”这个证据不存在。
///
/// 用普通 `acquire` 来源、非内核（动态 ASID）地址空间与显式 `attach_shaped` 造出
/// `G|U|A|D` 的叶子。这段映射从不装进任何 hart 的 `satp`，没有真缓存可言。
/// 同一页还有普通关系。发布时的永久来源标记在占槽前拒绝移交，
/// 两条 PTE 都必须保持不变；这里不单独证明后置预检分支。
fn a_global_mapping_is_not_transferable() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(1);
    let source = flat.whole();
    drop(source.materialize().expect("transfer: materialize source"));
    let address = placed(&source)[0].1;
    let target = acquire(1).whole();

    let global_space = SpaceBuilder::user()
        .build()
        .expect("transfer: global space");
    crate::expect!(
        !global_space.nest.asid().is_kernel(),
        "transfer: this relation lives in a dynamic user address space"
    );
    let reservation = support::reserve(&global_space, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("transfer: reserve");
    let attrs = PteFlags::G | PteFlags::U | PteFlags::A | PteFlags::D;
    let active = [
        ActiveRange::new(VirtAddr::wrap(BASE), PAGE_SIZE, Access::all(), rw)
            .expect("transfer: active range"),
    ];
    let global = global_space
        .nest
        .attach_shaped(reservation, source.clone(), attrs, &active)
        .expect("transfer: global relation");
    crate::expect!(
        global.is_global(),
        "transfer: the relation really carries a global entry"
    );
    // 普通那条后来登记，于是排在枚举前面：先被处理的就是它。
    let (plain_space, plain) = relation(&source, rw);
    let pressings =
        source_views::pressings_over(&source, PageIndex::ZERO).expect("transfer: pressings");
    crate::expect!(
        pressings.len() == 2
            && Arc::ptr_eq(&pressings[0].view, &plain)
            && !pressings[0].view.is_global()
            && pressings[1].view.is_global(),
        "transfer: the ordinary relation is enumerated before the global one"
    );
    for nest in [&plain_space.nest, &global_space.nest] {
        crate::expect!(
            nest.translate(VirtAddr::wrap(BASE))
                .map(|(physical, _)| physical)
                == Some(PhysAddr::from_raw(address)),
            "transfer: both relations start mapped"
        );
    }

    crate::expect!(
        transfer(&source, 0, &target, 0) == Err(TransferError::NotTransferable),
        "transfer: a global entry is a permanent refusal"
    );
    // 资格拒绝发生在占槽前，源、目标与两条 PTE 都不动。
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)],
        "transfer: a global refusal restores the source slot"
    );
    crate::expect!(
        placed(&target).is_empty(),
        "transfer: a global refusal leaves the seat empty"
    );
    for nest in [&plain_space.nest, &global_space.nest] {
        crate::expect!(
            nest.translate(VirtAddr::wrap(BASE))
                .map(|(physical, _)| physical)
                == Some(PhysAddr::from_raw(address)),
            "transfer: a permanent refusal leaves both mappings alone"
        );
        nest.audit();
    }
    drop(plain);
    drop(plain_space);
    drop(global);
    drop(global_space);
}

/// 全局条目发布过之后，整份来源永久不可迁——哪怕那条关系早就撤下、关联也清了。
///
/// 关联退场只说明“现在找不到这条关系”，不等于旧 TLB 里的全局条目没了；
/// 所以资格是来源自己的永久状态，不是按当前关系现算的。这里只造 PTE，
/// 从不把这段映射装进任何 hart 的 `satp`。
fn a_withdrawn_global_relation_keeps_the_source_untransferable() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let source = flat.whole();
    for index in 0..2 {
        let part = source.range(page(index), one()).expect("transfer: page");
        drop(part.materialize().expect("transfer: materialize page"));
    }
    let before = placed(&source);
    crate::expect!(before.len() == 2, "transfer: two carriers start in place");

    let global_space = SpaceBuilder::user()
        .build()
        .expect("transfer: global space");
    let reservation = support::reserve(&global_space, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("transfer: reserve");
    let attrs = PteFlags::G | PteFlags::U | PteFlags::A | PteFlags::D;
    let active = [
        ActiveRange::new(VirtAddr::wrap(BASE), PAGE_SIZE, Access::all(), rw)
            .expect("transfer: active range"),
    ];
    let global = global_space
        .nest
        .attach_shaped(reservation, slice(&source, 0, 1), attrs, &active)
        .expect("transfer: global relation");
    // 撤下这条关系并把它放掉：关联里再也不会出现它。
    global_space
        .nest
        .detach(global.range(page(0), one()).expect("transfer: selection"))
        .expect("transfer: withdraw the global relation");
    drop(global);
    drop(global_space);
    crate::expect!(
        source_views::pressings_over(&source, PageIndex::ZERO)
            .expect("transfer: pressings")
            .is_empty(),
        "transfer: no relation presses the source any more"
    );

    // 两页都拒：第一页曾进过全局映射，第二页从来没进过，也一样。
    let target = acquire(1).whole();
    for index in 0..2 {
        crate::expect!(
            transfer(&slice(&source, index, 1), 0, &target, 0)
                == Err(TransferError::NotTransferable),
            "transfer: a former global mapping keeps the whole source untransferable"
        );
    }
    crate::expect!(
        placed(&source) == before,
        "transfer: a permanent refusal changes no carrier"
    );
    crate::expect!(
        placed(&target).is_empty(),
        "transfer: a permanent refusal leaves the seat empty"
    );

    // 另一份普通来源照旧能迁：资格是每份来源自己的。
    let plain_flat = acquire(1);
    let plain_source = plain_flat.whole();
    let plain_page = plain_source
        .materialize()
        .expect("transfer: materialize plain");
    let plain_address = plain_page.physical(PageOffset::ZERO).expect("transfer: pa");
    drop(plain_page);
    let plain_target = acquire(1).whole();
    transfer(&plain_source, 0, &plain_target, 0).expect("transfer: an ordinary source still moves");
    crate::expect!(
        placed(&plain_target) == alloc::vec![(0, plain_address.as_usize())],
        "transfer: the ordinary carrier lands unchanged"
    );
}

/// 占槽之后、结算之前：别的驻留、物化（PA 出口）与发布都在同一个在途窗口里被拒。
///
/// 探针借真实在途窗口核对排除关系；这次移交自己随后照样成功：它把那一页撤下、
/// 把承载送到目标，而关系的身份与活动形状不变。
fn an_in_flight_claim_refuses_other_uses() {
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    drop(source.materialize().expect("transfer: materialize source"));
    let address = placed(&source)[0].1;
    write_byte(PhysAddr::from_raw(address), 0x11);

    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), whole.clone(), rw);
    let before = shape(&view);
    let seat = acquire(1);
    let seat_page = seat.whole();

    *PUBLISHED.lock() = Some((space.nest.clone(), view.clone(), view.whole()));
    crate::core::memory::transfer::arm_in_flight(Some(in_flight_refusals));
    let outcome = transfer(&source, 0, &seat_page, 0);
    crate::core::memory::transfer::arm_in_flight(None);
    *PUBLISHED.lock() = None;

    crate::expect!(
        outcome.is_ok(),
        "transfer: the in-flight window only excludes other uses"
    );
    // 承载与内容都到了目标；源页成了墓碑。
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address)],
        "transfer: the carrier reached the seat"
    );
    crate::expect!(
        read_byte(PhysAddr::from_raw(address)) == 0x11,
        "transfer: content survives the move"
    );
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Missing)),
        "transfer: the moved source page is a tombstone"
    );
    // 那一页的 PTE 没了；关系还在索引里，身份与活动形状不变；也不留驻留。
    crate::expect!(
        space.nest.translate(VirtAddr::wrap(BASE)).is_none(),
        "transfer: the moved page gives up its PTE"
    );
    crate::expect!(
        Arc::ptr_eq(
            &space
                .nest
                .probe(VirtAddr::wrap(BASE))
                .expect("transfer: the relation stays"),
            &view
        ),
        "transfer: the relation keeps its index entry"
    );
    crate::expect!(
        shape(&view) == before,
        "transfer: the relation keeps its identity and shape"
    );
    space.nest.audit();
    crate::expect!(
        !source.is_pinned(),
        "transfer: a move leaves no residency behind"
    );
    crate::expect!(
        !source.is_pinned(),
        "transfer: a move leaves no residency behind"
    );
    drop(
        source
            .reside()
            .expect("transfer: no residue blocks a new lease"),
    );
    drop(view);
    drop(space);
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
///
/// 这次的失败来自撤页之后的同步：撤页本身已经发生，回退只管两个页槽。
fn a_failed_move_keeps_a_tombstone_target() {
    let rw = Access::READ | Access::WRITE;
    let source_flat = acquire(1);
    let source = source_flat.whole();
    let pages = source.materialize().expect("transfer: materialize source");
    let physical = pages.physical(PageOffset::ZERO).expect("transfer: pa");
    let address = physical.as_usize();
    fill_pattern(physical);
    drop(pages);

    // 目标第 0 页先是墓碑：把它的承载先移到第 1 页去。
    let target_flat = acquire(2);
    let target = target_flat.whole();
    let tombstone = target.range(page(0), one()).expect("transfer: target page");
    drop(
        tombstone
            .materialize()
            .expect("transfer: materialize target"),
    );
    let other = target.range(page(1), one()).expect("transfer: empty seat");
    transfer(&tombstone, 0, &other, 0).expect("transfer: make a tombstone");
    crate::expect!(
        matches!(tombstone.materialize(), Err(FlatError::Missing)),
        "transfer: the target page is a tombstone"
    );

    // 这段来源上挂一段关系：这次移交会撤页，而它的同步被注入失败。
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    let carrier = Arc::downgrade(&source.materialized_frames().expect("transfer: slots")[0].1);
    fail_next_shootdown(space.nest.asid());
    crate::expect!(
        transfer(&source, 0, &tombstone, 0) == Err(TransferError::Busy),
        "transfer: the injected sync failure is retryable"
    );
    // 墓碑没被抹成 Fresh：物化仍然拒，而不是补出一个新承载。
    crate::expect!(
        matches!(tombstone.materialize(), Err(FlatError::Missing)),
        "transfer: a failed move keeps the target tombstone"
    );
    let slots = source
        .materialized_frames()
        .expect("transfer: source slots");
    crate::expect!(
        slots.len() == 1
            && Arc::ptr_eq(
                &slots[0].1,
                &carrier.upgrade().expect("transfer: carrier alive")
            ),
        "transfer: a failed move keeps the source carrier"
    );
    crate::expect!(
        pattern_holds(physical),
        "transfer: content survives the failure"
    );
    space
        .nest
        .retry_invalidation()
        .expect("transfer: settle the failed sync");
    drop(view);
    drop(space);
    // 结清之后真的移过去：目标从墓碑变成承载。
    transfer(&source, 0, &tombstone, 0).expect("transfer: move onto a tombstone");
    crate::expect!(
        placed(&tombstone) == alloc::vec![(0, address)],
        "transfer: a tombstone target accepts the carrier"
    );
    crate::expect!(
        pattern_holds(physical),
        "transfer: the move keeps the content"
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
    crate::expect!(
        seat_page.can_materialize(page(0)) == Ok(true),
        "transfer: a failed preparation leaves no Reserved seat"
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

/// 占槽前的候选准备失败：两端的槽、承载独占与关系清单都不动。
fn a_failed_candidate_preparation_changes_nothing() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    let pages = source.materialize().expect("transfer: materialize");
    let address = pages.physical(PageOffset::ZERO).expect("transfer: pa");
    drop(pages);
    let carrier = Arc::downgrade(&source.materialized_frames().expect("transfer: slots")[0].1);
    // 一段普通关系压在源页上：这次准备确实有候选要收。
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    let seat = acquire(2);
    let seat_page = seat.whole().range(page(1), one()).expect("transfer: seat");

    source_views::fail_candidates();
    let outcome = Transfer::new(source.clone(), page(0), seat_page.clone(), page(0))
        .expect("transfer: construct")
        .execute();
    crate::expect!(
        outcome.result == Err(TransferError::OutOfMemory),
        "transfer: a candidate preparation failure is reported"
    );
    crate::expect!(
        outcome.affected.is_empty(),
        "transfer: a pre-claim failure names no relation"
    );
    // 两端都没动：源还是同一承载、同一 PA；目标仍空。
    let slots = source
        .materialized_frames()
        .expect("transfer: source slots");
    crate::expect!(
        slots.len() == 1
            && Arc::ptr_eq(
                &slots[0].1,
                &carrier.upgrade().expect("transfer: carrier alive")
            ),
        "transfer: a candidate preparation failure keeps the source carrier"
    );
    crate::expect!(
        placed(&source) == alloc::vec![(0, address.as_usize())],
        "transfer: a candidate preparation failure changes no carrier"
    );
    crate::expect!(
        placed(&seat_page).is_empty(),
        "transfer: a candidate preparation failure leaves the seat empty"
    );
    crate::expect!(
        seat_page.can_materialize(page(0)) == Ok(true),
        "transfer: a candidate preparation failure leaves no Reserved seat"
    );
    // 承载独占也没被留下：随后一次真实移交照样取得它。
    transfer(&source, 0, &seat_page, 0).expect("transfer: the retry takes the carrier");
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Missing)),
        "transfer: the retry moved the carrier away"
    );
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, address.as_usize())],
        "transfer: the retry lands the same carrier"
    );
    drop(view);
    drop(space);
}

/// 占槽后的同步失败：两端先还原，清单仍交出两端真正动过的关系
/// （源页上的那条，与目标端那条惰性的）。
fn a_failed_move_still_names_both_ends() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    let outside = whole.range(page(1), one()).expect("transfer: outside");
    for index in 0..2 {
        let part = whole.range(page(index), one()).expect("transfer: part");
        drop(part.materialize().expect("transfer: materialize part"));
    }
    let carrier_address = placed(&source)[0].1;

    let left = SpaceBuilder::user().build().expect("transfer: left space");
    let alpha = map_slice(&left, VirtAddr::wrap(BASE), source.clone(), rw);
    // 同一份来源上还有一条压在**别的页**上的关系：它在候选里，却不是这次动过的。
    let beta = map_slice(&left, VirtAddr::wrap(BASE + PAGE_SIZE), outside.clone(), rw);

    // 目标端：一段惰性关系压着目标页（目标此刻是 Fresh，没有 PTE）。
    let target_flat = acquire(1);
    let target = target_flat.whole();
    let right = SpaceBuilder::user().build().expect("transfer: right space");
    let gamma = map_slice(&right, VirtAddr::wrap(BASE), target.clone(), rw);
    crate::expect!(
        right.nest.translate(VirtAddr::wrap(BASE)).is_none(),
        "transfer: the target relation starts lazy"
    );

    fail_next_shootdown(left.nest.asid());
    let outcome = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("transfer: construct")
        .execute();
    crate::expect!(
        outcome.result == Err(TransferError::Busy),
        "transfer: an unconfirmed sync is a retryable Busy"
    );
    // 两端先还原：源还是同一承载，目标仍空。
    crate::expect!(
        placed(&source) == alloc::vec![(0, carrier_address)],
        "transfer: a failed sync restores the source slot"
    );
    crate::expect!(
        placed(&target).is_empty(),
        "transfer: a failed sync leaves the target empty"
    );
    // 清单是两端真正动过的关系：源页上的 alpha 与目标页上的 gamma，窗口外的 beta 不在。
    crate::expect!(
        names(&outcome.affected, &alpha),
        "transfer: the source end is named"
    );
    crate::expect!(
        names(&outcome.affected, &gamma),
        "transfer: the lazy target end is named"
    );
    crate::expect!(
        !names(&outcome.affected, &beta),
        "transfer: a relation outside the window is not named"
    );
    crate::expect!(
        outcome.affected.len() == 2,
        "transfer: exactly the two live ends are named ({})",
        outcome.affected.len()
    );
    left.nest
        .retry_invalidation()
        .expect("transfer: settle the failed sync");
    let retried = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("transfer: construct retry")
        .execute();
    crate::expect!(
        retried.result.is_ok()
            && retried.affected.len() == 2
            && names(&retried.affected, &alpha)
            && names(&retried.affected, &gamma)
            && !names(&retried.affected, &beta),
        "transfer: a successful cross-source retry names exactly both ends"
    );
    crate::expect!(
        source.can_materialize(page(0)) == Ok(false)
            && placed(&target) == alloc::vec![(0, carrier_address)],
        "transfer: the successful retry settles both slots"
    );
    drop(beta);
    drop(alpha);
    drop(gamma);
    drop(left);
    drop(right);
    drop(target_flat);
}

/// 同一份来源的两页：同时盖住源页与目标页的那条关系只出现一次，
/// 窗口之外的候选不被误当成动过的。
fn a_relation_over_both_ends_is_named_once() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(3);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    let target = whole.range(page(1), one()).expect("transfer: target");
    let outside = whole.range(page(2), one()).expect("transfer: outside");
    drop(source.materialize().expect("transfer: materialize source"));
    drop(
        outside
            .materialize()
            .expect("transfer: materialize outside"),
    );
    let address = placed(&source)[0].1;

    let space = SpaceBuilder::user().build().expect("transfer: space");
    // 一段关系同时盖住源页与目标页（目标还是 Fresh，于是这一页是惰性的）。
    let both = map_slice(&space, VirtAddr::wrap(BASE), slice(&whole, 0, 2), rw);
    // 另一段只盖住窗口之外的第 2 页。
    let outside_view = map_slice(&space, VirtAddr::wrap(BASE + 2 * PAGE_SIZE), outside, rw);

    fail_next_shootdown(space.nest.asid());
    let failed = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("transfer: construct failing same-source move")
        .execute();
    crate::expect!(
        failed.result == Err(TransferError::Busy)
            && failed.affected.len() == 1
            && names(&failed.affected, &both)
            && !names(&failed.affected, &outside_view),
        "transfer: a failed same-source move names the shared relation once"
    );
    crate::expect!(
        placed(&source) == alloc::vec![(0, address)]
            && placed(&target).is_empty()
            && target.can_materialize(page(0)) == Ok(true),
        "transfer: the failed same-source move restores both slots"
    );
    space
        .nest
        .retry_invalidation()
        .expect("transfer: settle the failed same-source sync");

    // 失败时已撤 PTE；这次重试只结算槽与清单，不再证明一次新的撤页。
    let outcome = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("transfer: construct")
        .execute();
    crate::expect!(
        outcome.result.is_ok(),
        "transfer: the same-source move succeeds"
    );
    crate::expect!(
        outcome.affected.len() == 1 && names(&outcome.affected, &both),
        "transfer: a relation covering both ends is named once ({})",
        outcome.affected.len()
    );
    crate::expect!(
        !names(&outcome.affected, &outside_view),
        "transfer: a relation outside both pages is not named"
    );
    // 承载到了目标页；源页成墓碑；窗口之外那页没动。
    crate::expect!(
        placed(&target) == alloc::vec![(0, address)],
        "transfer: the carrier lands on the target page"
    );
    crate::expect!(
        matches!(source.materialize(), Err(FlatError::Missing)),
        "transfer: the source page is a tombstone"
    );
    crate::expect!(
        space.nest.translate(VirtAddr::wrap(BASE)).is_none(),
        "transfer: the shared relation gives up the source page PTE"
    );
    crate::expect!(
        space
            .nest
            .translate(VirtAddr::wrap(BASE + 2 * PAGE_SIZE))
            .is_some(),
        "transfer: the relation outside the window keeps its PTE"
    );
    space.nest.audit();
    drop(outside_view);
    drop(both);
    drop(space);
}

/// 只读可物化性：Fresh 与 Resident 为真，在途两端与墓碑为假，越界是独立错误。
fn can_materialize_reports_page_availability() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    let source = whole.range(page(0), one()).expect("transfer: source");
    let target = whole.range(page(1), one()).expect("transfer: target");

    // Fresh 可以，越界是独立的错。
    crate::expect!(
        source
            .can_materialize(PageOffset::ZERO)
            .expect("transfer: fresh"),
        "transfer: a fresh page can materialize"
    );
    crate::expect!(
        matches!(whole.can_materialize(page(2)), Err(FlatError::OutOfRange)),
        "transfer: an out-of-range page is its own error"
    );
    // Resident 可以。
    drop(source.materialize().expect("transfer: materialize source"));
    crate::expect!(
        source
            .can_materialize(PageOffset::ZERO)
            .expect("transfer: resident"),
        "transfer: a resident page can materialize"
    );

    // 一段关系压在源页上：这次移交会在撤页处失败并回退，回退后目标回到 Fresh。
    let space = SpaceBuilder::user().build().expect("transfer: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    *ENDS.lock() = Some((
        source.clone(),
        PageOffset::ZERO,
        target.clone(),
        PageOffset::ZERO,
    ));
    crate::core::memory::transfer::arm_in_flight(Some(can_materialize_ends));
    fail_next_shootdown(space.nest.asid());
    let outcome = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("transfer: construct")
        .execute();
    crate::core::memory::transfer::arm_in_flight(None);
    *ENDS.lock() = None;
    crate::expect!(
        outcome.result == Err(TransferError::Busy),
        "transfer: the injected sync failure is retryable"
    );
    // 回退之后：源页还是 Resident，目标页回到 Fresh，两者都可以物化。
    crate::expect!(
        source
            .can_materialize(PageOffset::ZERO)
            .expect("transfer: restored source"),
        "transfer: a rolled-back resident page can materialize"
    );
    crate::expect!(
        target
            .can_materialize(PageOffset::ZERO)
            .expect("transfer: restored target"),
        "transfer: a target rolled back to fresh can materialize"
    );
    // 真搬走一页之后，源页成墓碑：不能再物化；目标拿到承载就可以。
    space
        .nest
        .retry_invalidation()
        .expect("transfer: settle the failed sync");
    transfer(&source, 0, &target, 0).expect("transfer: the retry moves the carrier");
    crate::expect!(
        matches!(source.can_materialize(PageOffset::ZERO), Ok(false)),
        "transfer: a moved-out page cannot materialize"
    );
    crate::expect!(
        target
            .can_materialize(PageOffset::ZERO)
            .expect("transfer: target resident"),
        "transfer: the target page can materialize once it holds a carrier"
    );
    space.nest.audit();
    drop(view);
    drop(space);
}

/// 在途窗口里的可物化性：两端各自那一页都还不能物化。
fn can_materialize_ends(_source: &FlatRange, _page: PageIndex) {
    let ends = ENDS.lock();
    let (source, source_page, target, target_page) = ends.as_ref().expect("transfer: both ends");
    crate::expect!(
        matches!(source.can_materialize(*source_page), Ok(false)),
        "transfer: an in-flight source page cannot materialize"
    );
    crate::expect!(
        matches!(target.can_materialize(*target_page), Ok(false)),
        "transfer: an in-flight target page cannot materialize"
    );
}

/// 普通 `Acquire` 一整段 2 MiB 对齐窗口：移交只动中段那一格 4 KiB。
///
/// **这不是“大叶形成之后的完整移交”那一条判据**：物理分配只保证“页数 ＋ 页对齐”
/// （`Flat` 逐帧取自 buddy，且与块池的整页取自交织），本轮实测这一窗的 PA 首址不是
/// 2 MiB 对齐、相邻处有断点，`leaf_span` 两端都只报 4 KiB ⇒ 大叶前提在当前分配能力下
/// 没有合法构造，那一条仍**未验证**（实测读数见 docs/memory-transfer-traverse.md 该节）。
/// 本例**不把 4 KiB 钉成合同**：叶形是分配布局的事，连续对齐的分配会让这一窗变成
/// 2 MiB 叶——两种都由本例照过。
///
/// 本例声称的只是自己的事：起点是普通 `acquire` 的整段来源（不手工写 PTE、
/// 不借 `BorrowedRegion`、不直调表原语、不为它扩分配 API），在 2 MiB 对齐的 VA
/// 窗口上按页物化、从映射写内容，然后把中段那一格 4 KiB 交出去：只那一格让出翻译，
/// 邻页的承载、内容与翻译都不动。
fn a_plain_acquire_window_hands_over_one_page() {
    let window = MEGA / PAGE_SIZE;
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(window);
    let whole = flat.whole();
    let pages = whole
        .materialize()
        .expect("transfer: materialize the window");
    let mut addresses = Vec::new();
    addresses
        .try_reserve_exact(window)
        .expect("transfer: an address list");
    for index in 0..window {
        addresses.push(
            pages
                .physical(PageOffset::new(index))
                .expect("transfer: page address")
                .as_usize(),
        );
    }
    let carriers: Vec<_> = whole
        .materialized_frames()
        .expect("transfer: slots")
        .into_iter()
        .map(|(offset, frame)| (offset.get(), frame))
        .collect();
    drop(pages);
    crate::expect!(
        carriers.len() == window,
        "transfer: the whole window is materialized ({} of {window})",
        carriers.len()
    );
    crate::expect!(
        addresses.windows(2).all(|pair| pair[0] != pair[1]),
        "transfer: one carrier per page"
    );

    let space = SpaceBuilder::user()
        .build()
        .expect("transfer: window space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), whole.clone(), rw);
    // 整窗内容从**映射**写下去、再逐页读回来：每一页头一个字节就是它的页号，
    // 且每一页都翻到自己的承载上。叶形怎么落不在这里声称：当前布局只支持 4 KiB 叶，
    // 物理分配给出连续且对齐区段时会是 2 MiB 叶，两者都合法。
    for index in 0..window {
        let at = VirtAddr::wrap(BASE + index * PAGE_SIZE);
        crate::expect!(
            space.copy_out(&[index as u8], at),
            "transfer: write page {index} through the mapping"
        );
        let mut byte = [0u8; 1];
        crate::expect!(
            space.copy_in(&mut byte, at) && byte[0] == index as u8,
            "transfer: page {index} reads back through the mapping"
        );
        crate::expect!(
            space.nest.translate(at).expect("transfer: page").0
                == PhysAddr::from_raw(addresses[index]),
            "transfer: page {index} translates to its own carrier"
        );
    }

    let middle = window / 2;
    let moved = addresses[middle];
    let held = Arc::downgrade(
        &carriers
            .iter()
            .find(|(offset, _)| *offset == middle)
            .expect("transfer: the middle carrier")
            .1,
    );
    let seat = acquire(1);
    let seat_page = seat.whole();
    transfer(&slice(&whole, middle, 1), 0, &seat_page, 0)
        .expect("transfer: the middle 4 KiB page moves");

    // 目标拿到同一个承载、同一个 PA 与同一格内容。
    crate::expect!(
        placed(&seat_page) == alloc::vec![(0, moved)],
        "transfer: the moved page keeps its carrier"
    );
    crate::expect!(
        Arc::ptr_eq(
            &seat_page
                .materialized_frames()
                .expect("transfer: seat slots")[0]
                .1,
            &held.upgrade().expect("transfer: carrier alive")
        ),
        "transfer: the very same frame lands in the seat"
    );
    crate::expect!(
        read_byte(PhysAddr::from_raw(moved)) == middle as u8,
        "transfer: the moved page keeps its content"
    );
    crate::expect!(
        matches!(
            slice(&whole, middle, 1).materialize(),
            Err(FlatError::Missing)
        ),
        "transfer: the moved seat becomes a tombstone"
    );
    // 只有那一格让出翻译；邻页的承载、内容与翻译都不动。
    crate::expect!(
        space
            .nest
            .translate(VirtAddr::wrap(BASE + middle * PAGE_SIZE))
            .is_none(),
        "transfer: only the moved page gives up its translation"
    );
    // 跃下来的这一格没有 PTE 了，所以**没有哪一片叶能跨过它**：大叶必须先把这一格拆出来，
    // 拆开后邻页与窗两端就只能是 4 KiB 叶。这条与分配是否连续无关。
    for neighbour in [middle - 1, middle + 1] {
        let at = VirtAddr::wrap(BASE + neighbour * PAGE_SIZE);
        let mut byte = [0u8; 1];
        crate::expect!(
            space.copy_in(&mut byte, at) && byte[0] == neighbour as u8,
            "transfer: neighbour {neighbour} keeps its content"
        );
        crate::expect!(
            space.nest.translate(at).expect("transfer: neighbour").0
                == PhysAddr::from_raw(addresses[neighbour])
                && space.nest.leaf_span(at) == Some(PAGE_SIZE),
            "transfer: neighbour {neighbour} keeps its carrier, no leaf spans the hole"
        );
    }
    crate::expect!(
        space.nest.leaf_span(VirtAddr::wrap(BASE)) == Some(PAGE_SIZE)
            && space
                .nest
                .leaf_span(VirtAddr::wrap(BASE + MEGA - PAGE_SIZE))
                == Some(PAGE_SIZE),
        "transfer: no leaf spans the hole, so both window ends are 4 KiB leaves"
    );
    space.nest.audit();
    drop(view);
    drop(space);
    drop(seat);
}

/// 普通 `Flat::acquire` 的 Managed 来源，在**仅测试可控布局**下自然形成真实的 2 MiB 大叶，
/// 再把中段那一格 4 KiB 交出去。
///
/// 布局由 [`frame_layout::Reserved`] 塑形：先预热块池的两档元数据尺寸类（每页保留一块），
/// 再把帧池里小于 8 MiB 的伙伴块抽干并持有。此后一次 1024 页的普通物化整段落在相对连续
/// 的区段上，其中必含一个绝对 2 MiB 对齐的 512 页子窗——不手工写 PTE、不借
/// `BorrowedRegion`、不为它扩分配 API、不导入探针帧、也不盲试到偶然绿。
///
/// 这一层是**内核内的原生 Flat / Nest / Transfer**（真实来源、真实页表、真实事务），
/// 不是 guest ecall 那一层；单 Hart、单来源、单关系。
#[cfg(feature = "mega-leaf")]
pub fn mega_leaf() {
    use core::sync::atomic::Ordering;

    use crate::core::memory::nest::table::TABLE_LIVE;

    // 热身：首次建拆一个空间会触发一次性的惰性分配（核身份表与池的首次取自帧池），
    // 那不是本用例的账；先走一遍，再取基线。
    drop(
        SpaceBuilder::user()
            .build()
            .expect("mega-leaf: warm-up space"),
    );

    let frames_before = statistics::frame_occupied() - statistics::block_occupied();
    let kinds_before = statistics::kinds();
    let tables_before = TABLE_LIVE.load(Ordering::Relaxed);

    {
        const WINDOW: usize = 1024;
        const HALF: usize = 512;
        let rw = Access::READ | Access::WRITE;

        // 塑形：预热块池 → 建来源并预留页槽表 → 抽干帧池里的小块。
        let (flat, reserved) = match shaped_source(WINDOW) {
            Ok(pair) => pair,
            Err(why) => {
                crate::expect!(
                    false,
                    "mega-leaf: the controlled layout could not be shaped: {why}"
                );
                return;
            }
        };

        // 一次普通物化：整窗的 PA 与承载。
        let whole = flat.whole();
        let frames_at_shape = statistics::frame_occupied();
        let pages = whole
            .materialize()
            .expect("mega-leaf: materialize the window");
        let run_frames = statistics::frame_occupied() - frames_at_shape;
        let mut addresses = Vec::new();
        addresses
            .try_reserve_exact(WINDOW)
            .expect("mega-leaf: an address list");
        for index in 0..WINDOW {
            addresses.push(
                pages
                    .physical(PageOffset::new(index))
                    .expect("mega-leaf: page address")
                    .as_usize(),
            );
        }

        // 真实读数：整窗是一段相对连续段，且物化只取数据帧（元数据已预热）；其中有一个
        // 绝对 2 MiB 对齐、512 页连续的子窗，它的页偏移 = 2 MiB −（帧池基址 mod 2 MiB）。
        let contiguous = addresses
            .windows(2)
            .all(|pair| pair[1] == pair[0] + PAGE_SIZE);
        let first = (0..=WINDOW - HALF).find(|&i| {
            addresses[i] % MEGA == 0
                && (i..i + HALF).all(|k| addresses[k] == addresses[i] + (k - i) * PAGE_SIZE)
        });
        crate::putln!(
            "[health] mega-leaf: pool base {:#x}, {WINDOW} pages from {:#x} (contiguous {contiguous}), \
             {run_frames} frames for the run, aligned 2 MiB sub-window at {first:?} = {:#x}",
            crate::core::memory::flat::allocator::bump::frontier().next_multiple_of(PAGE_SIZE),
            addresses[0],
            first.map(|i| addresses[i]).unwrap_or(0)
        );
        crate::expect!(
            contiguous,
            "mega-leaf: the materialized window is one relative-contiguous run"
        );
        crate::expect!(
            run_frames == WINDOW,
            "mega-leaf: the run takes exactly {WINDOW} data frames ({run_frames}), no metadata frame"
        );
        let Some(first) = first else {
            crate::expect!(
                false,
                "mega-leaf: the controlled layout produced no aligned contiguous 2 MiB run"
            );
            return;
        };
        let carriers: Vec<_> = whole
            .materialized_frames()
            .expect("mega-leaf: slots")
            .into_iter()
            .map(|(offset, frame)| (offset.get(), frame))
            .collect();
        drop(pages);

        // 把这 512 页子窗挂到 2 MiB 对齐的 VA 窗口上。
        let source = whole
            .range(
                page(first),
                PageCount::new(HALF).expect("mega-leaf: sub-window"),
            )
            .expect("mega-leaf: sub-window");
        let space = SpaceBuilder::user().build().expect("mega-leaf: space");
        let view = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);

        // 真实大叶：整段是一级（2 MiB）叶；两端都在同一片叶里。
        for at in [BASE, BASE + MEGA - PAGE_SIZE] {
            crate::expect!(
                space.nest.leaf_span(VirtAddr::wrap(at)) == Some(MEGA),
                "mega-leaf: {at:#x} is in a real 2 MiB leaf"
            );
        }

        // 从映射写内容：每一页头一个字节就是它的页号。
        for index in 0..HALF {
            let at = VirtAddr::wrap(BASE + index * PAGE_SIZE);
            crate::expect!(
                space.copy_out(&[index as u8], at),
                "mega-leaf: write page {index} through the mapping"
            );
            let mut byte = [0u8; 1];
            crate::expect!(
                space.copy_in(&mut byte, at) && byte[0] == index as u8,
                "mega-leaf: page {index} reads back through the mapping"
            );
        }

        // 撤页前后都不许动的东西：窗两端与中段邻页的承载、权限。
        let watched = [0usize, HALF - 1, HALF / 2 - 1, HALF / 2 + 1];
        let kept: Vec<_> = watched
            .iter()
            .map(|&index| {
                let (physical, flags) = space
                    .nest
                    .translate(VirtAddr::wrap(BASE + index * PAGE_SIZE))
                    .expect("mega-leaf: watched page");
                (index, physical.as_usize(), flags.bits())
            })
            .collect();

        // 中段那一格 4 KiB 交出去。
        let middle = HALF / 2;
        let moved = addresses[first + middle];
        let held = Arc::downgrade(
            &carriers
                .iter()
                .find(|(offset, _)| *offset == first + middle)
                .expect("mega-leaf: the middle carrier")
                .1,
        );
        let seat = acquire(1);
        let seat_page = seat.whole();
        transfer(&source, middle, &seat_page, 0).expect("mega-leaf: the middle 4 KiB moves");

        // 目标拿到同一个承载、同一个 PA 与同一格内容。
        crate::expect!(
            placed(&seat_page) == alloc::vec![(0, moved)],
            "mega-leaf: the moved page keeps its carrier"
        );
        crate::expect!(
            Arc::ptr_eq(
                &seat_page
                    .materialized_frames()
                    .expect("mega-leaf: seat slots")[0]
                    .1,
                &held.upgrade().expect("mega-leaf: carrier alive")
            ),
            "mega-leaf: the very same frame lands in the seat"
        );
        crate::expect!(
            read_byte(PhysAddr::from_raw(moved)) == middle as u8,
            "mega-leaf: the moved page keeps its content"
        );
        crate::expect!(
            matches!(
                source
                    .range(page(middle), one())
                    .expect("mega-leaf: the moved cell")
                    .materialize(),
                Err(FlatError::Missing)
            ),
            "mega-leaf: the source cell becomes a tombstone"
        );
        crate::expect!(
            space
                .nest
                .translate(VirtAddr::wrap(BASE + middle * PAGE_SIZE))
                .is_none(),
            "mega-leaf: only the moved page gives up its translation"
        );
        // 拆叶真的发生了：邻页与窗两端从 2 MiB 叶变成 4 KiB 叶，承载、内容与权限都不动。
        for (index, physical, flags) in &kept {
            let at = VirtAddr::wrap(BASE + index * PAGE_SIZE);
            let mut byte = [0u8; 1];
            crate::expect!(
                space.copy_in(&mut byte, at) && byte[0] == *index as u8,
                "mega-leaf: page {index} keeps its content"
            );
            crate::expect!(
                space.nest.leaf_span(at) == Some(PAGE_SIZE),
                "mega-leaf: the 2 MiB leaf was split, page {index} is a 4 KiB leaf"
            );
            crate::expect!(
                space
                    .nest
                    .translate(at)
                    .expect("mega-leaf: kept page")
                    .0
                    .as_usize()
                    == *physical
                    && space
                        .nest
                        .translate(at)
                        .expect("mega-leaf: kept page")
                        .1
                        .bits()
                        == *flags,
                "mega-leaf: page {index} keeps its carrier and permissions"
            );
        }
        space.nest.audit();
        drop(view);
        drop(space);
        drop(seat);
        drop(source);
        drop(carriers);
        drop(addresses);
        drop(whole);
        drop(flat);
        drop(reserved);
    }

    let frames_after = statistics::frame_occupied() - statistics::block_occupied();
    crate::expect!(
        frames_after == frames_before,
        "mega-leaf: frame occupancy {frames_before} → {frames_after} (net must be 0)"
    );
    let tables_after = TABLE_LIVE.load(Ordering::Relaxed);
    crate::expect!(
        tables_after == tables_before,
        "mega-leaf: live tables {tables_before} → {tables_after} (net must be 0)"
    );
    for (kind, count) in statistics::kinds().nonzero() {
        if kind == Kind::Prime {
            // 块池取自帧池的整页；它们的账在 `block_occupied` 上，不在帧类目里。
            continue;
        }
        crate::expect!(
            count == kinds_before.get(kind),
            "mega-leaf: kind {}: {} → {count} (net must be 0)",
            kind.name(),
            kinds_before.get(kind)
        );
    }
}

/// 塑形并建出受控来源：预热块池、建来源、预留页槽表、抽干帧池里的小块。
///
/// 任一步失败都返回错误；这时 [`frame_layout::Reserved`] 已在作用域内，
/// 由它的析构把已持有的块整批归还，不留半个塑形。
#[cfg(feature = "mega-leaf")]
fn shaped_source(
    window: usize,
) -> Result<(Arc<Flat>, super::frame_layout::Reserved), &'static str> {
    use super::frame_layout;

    let mut reserved = frame_layout::Reserved::default();
    reserved.preheat()?;
    let allocation = Allocation::new(window, PAGE_SIZE).map_err(|_| "the window allocation")?;
    let flat = Flat::acquire(allocation).map_err(|_| "the source")?;
    flat.reserve_slots(window).map_err(|_| "the slot table")?;
    reserved.drain()?;
    Ok((flat, reserved))
}
