//! 用户页错误的纯探针与整段等待位。
//!
//! 这里的判据都是相对读数：本用例进入前后的帧占用与逐类净额必须回到基线。
//! 探针是只读的纯判据，本文件直调核心原语就能把四种结论与边界钉住；
//! 真实 Task 的等待-唤醒矩阵**尚未验**（测试上下文里没有可运行的任务），
//! 本文件不把它冒称已经验过。

#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, VirtAddr};
use crate::core::memory::flat::allocator::statistics;
use crate::core::memory::flat::allocator::statistics::Kind;
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatRange};
use crate::core::memory::nest::identity::fail_next_shootdown;
use crate::core::memory::nest::{Access, Asid, FaultKind, FaultOutcome, Nest, NestError, View};
use crate::core::memory::transfer::{Transfer, TransferError, arm_in_flight};
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::entry::trap::fault_wait::FaultWait;
use crate::machine::sync::SpinLock;

use super::support;

/// 本文件里关系的地址基址。
const BASE: usize = 0x6800_0000;

/// 在途探针要用的上下文：本次移交所在的空间与两端地址。
static IN_FLIGHT: SpinLock<Option<(Arc<Nest>, VirtAddr, VirtAddr)>> = SpinLock::new(None);

/// 在途探针要用的另一份上下文：一条**已经发布**的关系与它压着的那一页。
static IN_FLIGHT_PUBLISHED: SpinLock<Option<(Arc<Nest>, VirtAddr, usize)>> = SpinLock::new(None);

fn one() -> PageCount {
    PageCount::new(1).expect("fault: one page")
}

fn page(offset: usize) -> PageOffset {
    PageOffset::new(offset)
}

fn acquire(pages: usize) -> Arc<Flat> {
    Flat::acquire(Allocation::new(pages, PAGE_SIZE).expect("fault: allocation"))
        .expect("fault: acquire")
}

/// `range` 内从第 `from` 页起 `pages` 页的切片。
fn slice(range: &FlatRange, from: usize, pages: usize) -> FlatRange {
    range
        .range(page(from), PageCount::new(pages).expect("fault: pages"))
        .expect("fault: slice")
}

/// 把一段来源切片挂到 `space` 的 `at` 上；当前权限就是 `access`。
fn map_slice(space: &Space, at: VirtAddr, source: FlatRange, access: Access) -> Arc<View> {
    let size = source
        .page_count()
        .checked_bytes()
        .expect("fault: slice bytes");
    let reservation = support::reserve(space, at, size).expect("fault: reserve the slice");
    space
        .nest
        .attach(reservation, source, Access::all(), access)
        .expect("fault: attach the slice")
}

/// 等待位用例用的空间 ASID：同一段关系整段的键按 `Asid` 与关系身份定。
const KEY_SPACE: usize = 0x5A5A;

/// 纯探针的验收。
pub fn accept() {
    // 热身一：首次建拆一个空间会触发一次性的惰性分配，那不是本用例的账。
    drop(SpaceBuilder::user().build().expect("fault: warm-up space"));
    // 热身二：等待位用例要碰全局站点表，它的分片数组与哈希容量是一次性高水位，
    // 不属于本阶段的帧账；先在同一段空间上走一遍，再取基线。
    a_view_key_is_cleaned_with_its_space();

    let frames_before = statistics::frame_occupied() - statistics::block_occupied();
    let kinds_before = statistics::kinds();
    let cases: [(&str, fn()); 9] = [
        ("four outcomes", probe_reports_the_four_outcomes),
        ("in-flight ends", probe_waits_for_in_flight_ends),
        (
            "in-flight published",
            probe_waits_for_an_in_flight_source_with_a_live_entry,
        ),
        ("fresh rollback", probe_survives_a_failed_move),
        ("read-only probe", probe_is_read_only_until_settled),
        (
            "borrowed detach",
            a_borrowed_mapping_keeps_active_and_entry_in_step,
        ),
        ("wait predicate", the_wait_predicate_names_one_relation),
        ("resource failure", a_resource_failure_is_reported),
        ("space entries", space_entries_reach_the_nest),
    ];
    let mut off = Vec::new();
    for (name, run) in cases {
        let frames = statistics::frame_occupied() - statistics::block_occupied();
        let kinds = statistics::kinds();
        run();
        let mut same = statistics::frame_occupied() - statistics::block_occupied() == frames;
        for (kind, count) in statistics::kinds().nonzero() {
            if kind == Kind::Prime {
                continue;
            }
            same &= count == kinds.get(kind);
        }
        if !same {
            off.push(name);
        }
        super::report_ok(name, format_args!("probe"));
    }
    crate::expect!(
        off.is_empty(),
        "fault: cases that do not net out: {:?}",
        off
    );

    let frames_after = statistics::frame_occupied() - statistics::block_occupied();
    crate::expect!(
        frames_after == frames_before,
        "fault: frame occupation returns to baseline ({frames_before} -> {frames_after})"
    );
    let kinds_after = statistics::kinds();
    for (kind, count) in kinds_after.nonzero() {
        if kind == Kind::Prime {
            // 块池取自帧池的整页；它们的账在 `block_occupied` 上，不在帧类目里。
            continue;
        }
        crate::expect!(
            count == kinds_before.get(kind),
            "fault: kind {}: {} -> {count} (net must be 0)",
            kind.name(),
            kinds_before.get(kind)
        );
    }
}

/// 四种结论：可续、可物化、只能等、不该续。
fn probe_reports_the_four_outcomes() {
    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user().build().expect("fault: space");

    // Resolved：承载已发布，硬件条目已在且满足这次访问。
    let resident = acquire(1);
    drop(
        slice(&resident.whole(), 0, 1)
            .materialize()
            .expect("fault: materialize resident"),
    );
    let resolved = map_slice(
        &space,
        VirtAddr::wrap(BASE),
        slice(&resident.whole(), 0, 1),
        rw,
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Resolved,
        "fault: a published page already resolves a load"
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Store)
            == FaultOutcome::Resolved,
        "fault: a published writable page resolves a store"
    );

    // Materialize：Fresh 的页没有条目，但承载可得。
    let fresh = acquire(1);
    let materialize = map_slice(
        &space,
        VirtAddr::wrap(BASE + PAGE_SIZE),
        slice(&fresh.whole(), 0, 1),
        rw,
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: a fresh page has a carrier to materialize"
    );

    // Wait：承载已经移走，留下墓碑。
    let moved = acquire(2);
    drop(
        slice(&moved.whole(), 0, 2)
            .materialize()
            .expect("fault: materialize moved"),
    );
    let seat = acquire(1);
    Transfer::new(
        slice(&moved.whole(), 0, 1),
        page(0),
        slice(&seat.whole(), 0, 1),
        page(0),
    )
    .expect("fault: transfer")
    .execute()
    .result
    .expect("fault: move the carrier out");
    let waiting = map_slice(
        &space,
        VirtAddr::wrap(BASE + 2 * PAGE_SIZE),
        slice(&moved.whole(), 0, 1),
        rw,
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + 2 * PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Wait(waiting.identity()),
        "fault: a page whose carrier is gone waits on its relation"
    );

    // Invalid：活动权限不容纳这次访问。
    let readonly = map_slice(
        &space,
        VirtAddr::wrap(BASE + 3 * PAGE_SIZE),
        slice(&fresh.whole(), 0, 1),
        Access::READ,
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + 3 * PAGE_SIZE), FaultKind::Store)
            == FaultOutcome::Invalid,
        "fault: a store on a read-only range is invalid"
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + 3 * PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: the same range still loads"
    );

    // Invalid：地址不在任何关系里（越界或守卫）。
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + 8 * PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Invalid,
        "fault: an address in no relation is invalid"
    );

    space.nest.audit();
    drop(readonly);
    drop(waiting);
    drop(materialize);
    drop(resolved);
    drop(seat);
    drop(moved);
    drop(fresh);
    drop(resident);
    drop(space);
}

/// 在途的两端都只能等：源页 `Moving`、目标页 `Reserved`。
/// 两条关系都还没发布任何条目（惰性）；旧条目仍在的那一档见下一个用例。
fn probe_waits_for_in_flight_ends() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    let source = slice(&whole, 0, 1);
    let target = slice(&whole, 1, 1);
    let space = SpaceBuilder::user().build().expect("fault: space");
    // 两条惰性关系：源页此刻还没有承载，于是挂载时不发布任何条目。
    let over_source = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    let over_target = map_slice(&space, VirtAddr::wrap(BASE + PAGE_SIZE), target.clone(), rw);
    // 给源页配上承载：物化不动关系，关系这一页仍然没有条目。
    drop(source.materialize().expect("fault: materialize the source"));
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: materializing a source does not publish a lazy relation"
    );

    *IN_FLIGHT.lock() = Some((
        space.nest.clone(),
        VirtAddr::wrap(BASE),
        VirtAddr::wrap(BASE + PAGE_SIZE),
    ));
    arm_in_flight(Some(probe_in_flight_ends));
    let outcome = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("fault: transfer")
        .execute();
    arm_in_flight(None);
    *IN_FLIGHT.lock() = None;
    outcome.result.expect("fault: the in-flight move succeeds");

    // 迁移之后：源页成墓碑（只能等），目标页拿到承载（可以物化）。
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Wait(over_source.identity()),
        "fault: the source page waits once its carrier is moved out"
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: the target page can materialize the carrier it received"
    );
    space.nest.audit();
    drop(over_target);
    drop(over_source);
    drop(space);
    drop(flat);
}

/// 在途窗口里的两端都只能等。
fn probe_in_flight_ends(_source: &FlatRange, _page: PageIndex) {
    let ends = IN_FLIGHT.lock();
    let (nest, source, target) = ends.as_ref().expect("fault: both ends");
    crate::expect!(
        matches!(
            nest.probe_fault(*source, FaultKind::Load),
            FaultOutcome::Wait(_)
        ),
        "fault: an in-flight source page waits"
    );
    crate::expect!(
        matches!(
            nest.probe_fault(*target, FaultKind::Load),
            FaultOutcome::Wait(_)
        ),
        "fault: an in-flight target page waits"
    );
}

/// 回退之后：目标回到 Fresh、源回到 Resident，两边都照样可以物化；
/// 完整解析能把撤掉的那一页重新发布出来。
fn probe_survives_a_failed_move() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    let source = slice(&whole, 0, 1);
    let target = slice(&whole, 1, 1);
    drop(source.materialize().expect("fault: materialize the source"));
    let space = SpaceBuilder::user().build().expect("fault: space");
    // 源页上的关系已发布（撤页会走到确认那一步）；目标页上的关系是惰性的。
    let over_source = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    let over_target = map_slice(&space, VirtAddr::wrap(BASE + PAGE_SIZE), target.clone(), rw);
    fail_next_shootdown(space.nest.asid());
    let outcome = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("fault: transfer")
        .execute();
    crate::expect!(
        outcome.result == Err(TransferError::Busy),
        "fault: an unconfirmed sync rolls the move back"
    );

    // 目标槽回到 Fresh：它仍然可以物化，不会永远等下去。
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: a target rolled back to fresh can still materialize"
    );
    // 源页回到 Resident：也可以再物化一次。
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: a source restored to resident can materialize again"
    );
    // 完整解析（提交边界之外）：结清未确认的失效，把这一页重新发布。
    crate::expect!(
        space
            .nest
            .resolve_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == Ok(FaultOutcome::Resolved),
        "fault: the full resolver republishes the restored source page"
    );
    space.nest.audit();
    drop(over_target);
    drop(over_source);
    drop(space);
    drop(flat);
}

/// 纯探针不结算待办失效、也不分配：它只读当前状态，而完整解析才结清并重判。
fn probe_is_read_only_until_settled() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    drop(
        slice(&whole, 0, 2)
            .materialize()
            .expect("fault: materialize"),
    );
    let space = SpaceBuilder::user().build().expect("fault: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), slice(&whole, 0, 2), rw);

    // Fresh 的页：探针不物化，探完之后页槽还是空的。
    let fresh = acquire(1);
    let fresh_view = map_slice(
        &space,
        VirtAddr::wrap(BASE + 2 * PAGE_SIZE),
        slice(&fresh.whole(), 0, 1),
        rw,
    );
    let frames = statistics::frame_occupied();
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + 2 * PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: a fresh page reports materializable"
    );
    crate::expect!(
        statistics::frame_occupied() == frames
            && slice(&fresh.whole(), 0, 1)
                .materialized_frames()
                .expect("fault: slots")
                .is_empty(),
        "fault: the probe leaves a fresh page unmaterialized"
    );

    // 一次局部撤下：注入的同步失败把这次失效留成未结算。
    fail_next_shootdown(space.nest.asid());
    crate::expect!(
        space
            .nest
            .detach(view.range(page(0), one()).expect("fault: selection"))
            == Err(NestError::Shootdown),
        "fault: the injected sync failure leaves an unsettled invalidation"
    );
    crate::expect!(
        space.nest.has_pending_invalidation(),
        "fault: the nest really has an unsettled invalidation"
    );

    // 探针照旧给出结论：既不结算那次失效，也不分配。仍然可用的 PTE 不能
    // 自己当作“已经解决”——那份失效未确认，必须先结清再判。
    let frames = statistics::frame_occupied();
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == FaultOutcome::Materialize,
        "fault: a usable PTE does not resolve while an invalidation is unsettled"
    );
    crate::expect!(
        space.nest.has_pending_invalidation(),
        "fault: the probe does not settle the invalidation"
    );
    crate::expect!(
        statistics::frame_occupied() == frames,
        "fault: the probe allocates no frame"
    );

    // 完整解析在提交边界外先结清，再重判：现在才是真的可继续。
    crate::expect!(
        space
            .nest
            .resolve_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == Ok(FaultOutcome::Resolved),
        "fault: the full resolver settles and then resolves"
    );
    crate::expect!(
        !space.nest.has_pending_invalidation(),
        "fault: the full resolver settled the invalidation"
    );

    space
        .nest
        .retry_invalidation()
        .expect("fault: settle the invalidation");
    space.nest.audit();
    drop(fresh_view);
    drop(view);
    drop(space);
    drop(flat);
}

/// 在途的源页即使旧条目还在，也按已定的四态合同只等不判“已解”。
fn probe_waits_for_an_in_flight_source_with_a_live_entry() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    let source = slice(&whole, 0, 1);
    let target = slice(&whole, 1, 1);
    drop(source.materialize().expect("fault: materialize the source"));
    let space = SpaceBuilder::user().build().expect("fault: space");
    // 已经发布的关系：这一页此刻既有承载也有条目。
    let published = map_slice(&space, VirtAddr::wrap(BASE), source.clone(), rw);
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Resolved,
        "fault: a published page resolves before the move"
    );

    *IN_FLIGHT_PUBLISHED.lock() = Some((
        space.nest.clone(),
        VirtAddr::wrap(BASE),
        published.identity(),
    ));
    arm_in_flight(Some(probe_waits_while_the_entry_stands));
    let outcome = Transfer::new(source.clone(), page(0), target.clone(), page(0))
        .expect("fault: transfer")
        .execute();
    arm_in_flight(None);
    *IN_FLIGHT_PUBLISHED.lock() = None;
    outcome.result.expect("fault: the in-flight move succeeds");

    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Wait(published.identity()),
        "fault: the source page waits once its carrier is moved out"
    );
    space.nest.audit();
    drop(published);
    drop(space);
    drop(flat);
}

/// 在途窗口里：旧条目还在，探针也只按四态合同给 `Wait`。
fn probe_waits_while_the_entry_stands(_source: &FlatRange, _page: PageIndex) {
    let ends = IN_FLIGHT_PUBLISHED.lock();
    let (nest, address, expected) = ends.as_ref().expect("fault: the published relation");
    crate::expect!(
        nest.translate(*address).is_some(),
        "fault: the old entry is still standing in the in-flight window"
    );
    crate::expect!(
        nest.probe_fault(*address, FaultKind::Load) == FaultOutcome::Wait(*expected),
        "fault: an in-flight source waits even with its old entry standing"
    );
}

/// 借用来源的映射不会留下“active 还在、条目已被撤”的半截状态：
/// 撤页与活动库存同锁更新，未结算只能先结清；结清后被删的那一页不再属于任何关系。
fn a_borrowed_mapping_keeps_active_and_entry_in_step() {
    let rw = Access::READ | Access::WRITE;
    // 借用区间指向一块真在手上的物理存储：这里只做地址算术，不解引用。
    let backing = acquire(2);
    let base = slice(&backing.whole(), 0, 2)
        .materialize()
        .expect("fault: backing")
        .physical(PageOffset::ZERO)
        .expect("fault: backing pa");
    let region = unsafe { BorrowedRegion::new(base, 2 * PAGE_SIZE) }.expect("fault: borrowed");
    let borrowed = Flat::from_region(region)
        .expect("fault: borrowed source")
        .whole();
    let space = SpaceBuilder::user().build().expect("fault: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), borrowed, rw);

    // 借用的关系挂载时整段发布：两页此刻都可用。
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Resolved
            && space
                .nest
                .probe_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
                == FaultOutcome::Resolved,
        "fault: a borrowed relation publishes every active page"
    );

    // 只撤第 0 页，并让这次失效未结算。
    fail_next_shootdown(space.nest.asid());
    crate::expect!(
        space
            .nest
            .detach(view.range(page(0), one()).expect("fault: selection"))
            == Err(NestError::Shootdown),
        "fault: the injected sync failure leaves an unsettled invalidation"
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Materialize
            && space
                .nest
                .probe_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
                == FaultOutcome::Materialize,
        "fault: an unsettled invalidation goes through the resolver"
    );

    // 结清这一步自己失败：交回资源失败，不把借用页伪成可物化。
    fail_next_shootdown(space.nest.asid());
    crate::expect!(
        space
            .nest
            .resolve_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == Err(NestError::Shootdown),
        "fault: the borrowed case reports the real settle failure"
    );
    space
        .nest
        .retry_invalidation()
        .expect("fault: settle the invalidation");

    // 结清之后：被撤的那一页已经不在任何关系里，邻居照旧可用。
    crate::expect!(
        space
            .nest
            .resolve_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == Ok(FaultOutcome::Invalid),
        "fault: a withdrawn borrowed page is no longer mapped"
    );
    crate::expect!(
        space
            .nest
            .resolve_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == Ok(FaultOutcome::Resolved),
        "fault: the borrowed neighbour still resolves"
    );
    space.nest.audit();
    drop(view);
    drop(space);
    drop(backing);
}

/// 一段“只能等”的来源：第 0 页的承载已被移走，留下墓碍；承载随 `seat` 一起
/// 交回调用方，免得它被提前回收。返回那一段墓碑来源与承载所在的那一段。
fn tombstone(pages: usize) -> (FlatRange, FlatRange) {
    let flat = acquire(pages);
    let whole = flat.whole();
    drop(
        slice(&whole, 0, pages)
            .materialize()
            .expect("fault: materialize"),
    );
    let seat = acquire(1);
    Transfer::new(
        slice(&whole, 0, 1),
        page(0),
        slice(&seat.whole(), 0, 1),
        page(0),
    )
    .expect("fault: transfer")
    .execute()
    .result
    .expect("fault: move the carrier out");
    (slice(&whole, 0, 1), seat.whole())
}

/// 等待位只能挂在它自己认的那段关系上：同一 VA 换了关系，旧身份必须立刻重探。
fn the_wait_predicate_names_one_relation() {
    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user().build().expect("fault: space");
    let (source, seat_a) = tombstone(2);
    let old = map_slice(&space, VirtAddr::wrap(BASE), source, rw);
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Wait(old.identity()),
        "fault: a tombstone relation waits"
    );

    // 同一 VA 换一段关系：旧身份退场（它的键已经被 signal 过），新身份也在等。
    space
        .nest
        .detach(old.whole())
        .expect("fault: detach the first relation");
    let (replacement, seat_b) = tombstone(2);
    let new = map_slice(&space, VirtAddr::wrap(BASE), replacement, rw);
    crate::expect!(
        new.identity() != old.identity(),
        "fault: the replacement carries its own identity"
    );
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Load)
            == FaultOutcome::Wait(new.identity()),
        "fault: the replacement waits as well"
    );

    // 认旧身份的等待位直接放行回去重探；认新身份的才是真的可以睡。
    crate::expect!(
        FaultWait::new(
            space.nest.clone(),
            VirtAddr::wrap(BASE),
            FaultKind::Load,
            old.identity(),
        )
        .ready(),
        "fault: a replaced identity must not sleep on the old key"
    );
    crate::expect!(
        !FaultWait::new(
            space.nest.clone(),
            VirtAddr::wrap(BASE),
            FaultKind::Load,
            new.identity(),
        )
        .ready(),
        "fault: the live identity is worth sleeping on"
    );

    drop(new);
    drop(old);
    drop(seat_b);
    drop(seat_a);
    drop(space);
}

/// 真实资源失败（未确认的同步）照实交回，不折进四种结论之一。
fn a_resource_failure_is_reported() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(2);
    let whole = flat.whole();
    drop(
        slice(&whole, 0, 2)
            .materialize()
            .expect("fault: materialize"),
    );
    let space = SpaceBuilder::user().build().expect("fault: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), slice(&whole, 0, 2), rw);
    // 先制造一件未结算的失效，再让“结清”这一步自己失败。
    fail_next_shootdown(space.nest.asid());
    crate::expect!(
        space
            .nest
            .detach(view.range(page(0), one()).expect("fault: selection"))
            == Err(NestError::Shootdown),
        "fault: the first injection leaves an unsettled invalidation"
    );
    fail_next_shootdown(space.nest.asid());
    crate::expect!(
        space
            .nest
            .resolve_fault(VirtAddr::wrap(BASE + PAGE_SIZE), FaultKind::Load)
            == Err(NestError::Shootdown),
        "fault: an unconfirmed sync is reported as a resource failure"
    );
    // 失败的责任仍留在原处：同一身份重试可以结清。
    space
        .nest
        .retry_invalidation()
        .expect("fault: settle the invalidation");
    space.nest.audit();
    drop(view);
    drop(space);
    drop(flat);
}

/// 一段关系的等待位与它所在的空间一起清理：空间清场不能漏下内部键。
fn a_view_key_is_cleaned_with_its_space() {
    let asid = Asid::from_raw(KEY_SPACE);
    let life = Life::try_new().expect("fault: life");
    let weak = Arc::downgrade(&life);
    // 先拿四条同分片的键把分片表撞开：它的容量高水位是一次性的，不算这两条的账。
    for view in 0..4usize {
        messenger::knock(WakeKey::View { space: asid, view }, &weak);
    }
    messenger::wipe_space(asid);
    let before = messenger::site_count();
    for view in [0x11usize, 0x22] {
        messenger::knock(WakeKey::View { space: asid, view }, &weak);
    }
    crate::expect!(
        messenger::site_count() == before + 2,
        "fault: a view key takes a wait site"
    );
    messenger::wipe_space(asid);
    crate::expect!(
        messenger::site_count() == before,
        "fault: wiping a space drops its view keys"
    );
    drop(life);
}

/// 空间层的四个入口把结果原样交给 Nest；放行等待者的 signal 在无等待者时是空放行，
/// 这里只核对入口自己与 Nest 观测一致（真实唤醒链未验）。
fn space_entries_reach_the_nest() {
    let rw = Access::READ | Access::WRITE;
    let flat = acquire(1);
    let whole = flat.whole();
    drop(
        slice(&whole, 0, 1)
            .materialize()
            .expect("fault: materialize"),
    );
    let space = SpaceBuilder::user().build().expect("fault: space");
    let view = map_slice(&space, VirtAddr::wrap(BASE), slice(&whole, 0, 1), rw);

    space
        .narrow(view.whole(), Access::READ)
        .expect("fault: narrow through the space");
    crate::expect!(
        space
            .nest
            .probe_fault(VirtAddr::wrap(BASE), FaultKind::Store)
            == FaultOutcome::Invalid,
        "fault: the narrowed range rejects a store"
    );
    crate::expect!(
        space.access(view.whole(), rw) == Err(NestError::WidenDenied),
        "fault: the space entry reports the nest's refusal"
    );
    space
        .withdraw(view.whole())
        .expect("fault: withdraw through the space");
    crate::expect!(
        !space.nest.contains_reservation(view.identity()),
        "fault: the space entry really withdrew the range"
    );
    drop(view);
    drop(space);
    drop(flat);
}
