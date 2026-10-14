#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec;

use ::core::sync::atomic::Ordering;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, PhysAddr, VirtAddr};
use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
use crate::core::memory::flat::{BorrowedRegion, Frame};
use crate::core::memory::nest::table::{PteFlags, TABLE_LIVE};
use crate::core::memory::nest::{
    Access, NestError, Placement, RegionId, View, ViewRange, identity,
};
use crate::core::work::unit::space::{Space, SpaceBuilder};

const BASE: usize = 0x6000_0000;
const MEGA: usize = 1 << 21;

fn rw() -> Access {
    Access::READ | Access::WRITE
}

/// 一段页对齐窗口在某个关系上的身份选择。
fn select(view: &Arc<View>, start: VirtAddr, size: usize) -> ViewRange {
    let offset = (start.as_usize() - view.initial_start().as_usize()) / PAGE_SIZE;
    view.range(
        PageOffset::new(offset),
        PageCount::new(size / PAGE_SIZE).expect("nest: page count"),
    )
    .expect("nest: selection")
}

/// 在 `start` 上占一段并挂上既有帧。
fn frame_view(
    space: &Space,
    start: usize,
    frame: Arc<Frame>,
) -> crate::core::work::unit::space::Mapping {
    space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(start), PAGE_SIZE)
                .expect("nest: reservation"),
            vec![frame],
            rw(),
            rw(),
        )
        .expect("nest: map frames")
}

/// 地址原语的健康检查：登记不挂页、切分不丢段、顶页、选择与身份、
/// 改权同步失败、模块期地址空间、大叶分裂、非连续帧与缺页发布。
pub fn nest_views() {
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    clean_commit_reads_back();
    reserve_installs_no_pte();
    guard_split_keeps_both_spans();
    top_page_exact_and_split();
    detached_selection_and_closed_space();
    access_failure_keeps_the_relation_open();
    kernel_space_is_pinned();
    large_leaf_splits_on_access_change();
    noncontiguous_frames_translate_per_page();
    lazy_page_is_published_one_at_a_time();
    derive_publishes_source_window_and_shares_frames();
    per_page_current_copies_and_boundary();
    shaped_attach_keeps_holes();
    partial_detach_returns_only_that_span();
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

/// 成功挂载之后读回的是干净提交：身份、根页表、几何、待失效与待收尾都归位。
fn clean_commit_reads_back() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let frame = Frame::allocate().expect("nest: frame");
    let weak = Arc::downgrade(&frame);
    let view = frame_view(&space, BASE, frame);
    assert!(!nest.asid().is_kernel());
    assert_ne!(nest.asid().get(), 0);
    assert_ne!(nest.root_ppn(), 0);
    assert!(nest.geometry().levels >= 3);
    assert!(!nest.has_pending_invalidation());
    assert!(nest.pending_views().expect("nest: pending").is_empty());
    assert_eq!(
        view.view().initial_range(),
        (VirtAddr::wrap(BASE), PAGE_SIZE)
    );
    assert_eq!(nest.view_count(), 2);
    space.memory.unmap(&view).expect("nest: release");
    // 撤销只撤 PTE：来源仍强持这一页。
    assert!(weak.upgrade().is_some());
    assert!(!view.view().is_live());
    drop(view);
    drop(space);
    assert!(weak.upgrade().is_none());
}

/// 派生先让 `Flat` 物化源活动片段，再回到源关系发布；两边共享同一批帧。
fn derive_publishes_source_window_and_shares_frames() {
    const TARGET: usize = BASE + 0x20_000;
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let lazy = space
        .memory
        .map_lazy(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), 2 * PAGE_SIZE)
                .expect("nest: reservation"),
            rw(),
            rw(),
        )
        .expect("nest: lazy source");
    // 源还没物化：两页都没有翻译。
    assert!(nest.translate(VirtAddr::wrap(BASE)).is_none());
    assert!(nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    let derived = space
        .memory
        .view_record(
            lazy.view(),
            0,
            2 * PAGE_SIZE,
            PAGE_SIZE,
            rw(),
            rw(),
            Some(VirtAddr::wrap(TARGET)),
        )
        .expect("nest: derived view");
    // 旧副作用：源活动片段被物化并发布，两边都翻译得到。
    for at in [BASE, BASE + PAGE_SIZE, TARGET, TARGET + PAGE_SIZE] {
        assert!(nest.translate(VirtAddr::wrap(at)).is_some(), "{at:#x}");
    }
    let source_frames = lazy
        .view()
        .frames_in(VirtAddr::wrap(BASE), 2 * PAGE_SIZE)
        .expect("nest: source frames");
    let target_frames = derived
        .view()
        .frames_in(VirtAddr::wrap(TARGET), 2 * PAGE_SIZE)
        .expect("nest: target frames");
    assert_eq!(source_frames.len(), 2);
    assert_eq!(target_frames.len(), 2);
    for ((_, source), (_, target)) in source_frames.iter().zip(&target_frames) {
        assert!(
            Arc::ptr_eq(source, target),
            "derived view shares the source frame"
        );
    }
    assert_eq!(
        nest.translate(VirtAddr::wrap(BASE))
            .expect("nest: source page")
            .0,
        nest.translate(VirtAddr::wrap(TARGET))
            .expect("nest: target page")
            .0,
    );
    // 同源物理页：写一侧，另一侧读得到。
    assert!(space.memory.copy_out(b"shared", TARGET));
    let mut bytes = [0u8; 6];
    assert!(space.memory.copy_in(&mut bytes, BASE));
    assert_eq!(&bytes, b"shared");
    nest.audit();
    space.memory.unmap(&derived).expect("nest: release derived");
    space.memory.unmap(&lazy).expect("nest: release lazy");
}

/// 同一关系里两页当前权限不同：逐页访问、跨页复制与派生都按各自 current 判。
fn per_page_current_copies_and_boundary() {
    const SPAN: usize = 2 * PAGE_SIZE;
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let frames = vec![
        Frame::allocate().expect("nest: frame a"),
        Frame::allocate().expect("nest: frame b"),
    ];
    let view = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), SPAN)
                .expect("nest: reservation"),
            frames,
            rw(),
            rw(),
        )
        .expect("nest: map two pages");
    nest.set_access(
        select(view.view(), VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE),
        Access::READ,
    )
    .expect("nest: make the second page read-only");
    let first = VirtAddr::wrap(BASE);
    let second = VirtAddr::wrap(BASE + PAGE_SIZE);
    assert!(nest.check_access(first, PAGE_SIZE, Access::WRITE));
    assert!(!nest.check_access(second, PAGE_SIZE, Access::WRITE));
    assert!(nest.check_access(second, PAGE_SIZE, Access::READ));
    assert!(space.memory.copy_out(b"left", BASE));
    let mut bytes = [0u8; 4];
    assert!(space.memory.copy_in(&mut bytes, BASE + PAGE_SIZE));
    assert_eq!(&bytes, b"\0\0\0\0");
    // 跨到只读页的写整段拒；只写第一页可以。
    let zeros = [0u8; SPAN];
    assert!(!nest.copy_to(first, &zeros, Access::WRITE));
    assert!(nest.copy_to(first, &zeros[..PAGE_SIZE], Access::WRITE));
    let mut whole = [0u8; SPAN];
    assert!(nest.copy_from(first, &mut whole, Access::READ));
    // 上限收窄之外不能加宽；架构属性不受改权影响。
    assert_eq!(
        nest.set_access(
            select(view.view(), first, SPAN),
            Access::READ | Access::WRITE | Access::EXECUTE,
        ),
        Err(NestError::WidenDenied)
    );
    let flags = nest.translate(second).expect("nest: read-only page").1;
    assert!(flags.contains(PteFlags::U | PteFlags::R | PteFlags::A | PteFlags::D));
    assert!(!flags.contains(PteFlags::W));
    // 派生按请求给的 current / ceiling，不继承源页权限。
    let derived = space
        .memory
        .view_record(
            view.view(),
            0,
            SPAN,
            PAGE_SIZE,
            Access::READ | Access::EXECUTE,
            Access::READ,
            None,
        )
        .expect("nest: derived read-only view");
    for range in derived.view().active().expect("nest: derived active") {
        assert_eq!(range.current, Access::READ);
        assert_eq!(range.ceiling, Access::READ | Access::EXECUTE);
    }
    assert!(nest.check_access(derived.address(), PAGE_SIZE, Access::READ));
    assert!(!nest.check_access(derived.address(), PAGE_SIZE, Access::WRITE));
    nest.audit();
    space.memory.unmap(&derived).expect("nest: release derived");
    space.memory.unmap(&view).expect("nest: release view");
}

/// 活动形状带洞：洞不发布，活动库存就是两个片段，整段重撤不会去补洞。
fn shaped_attach_keeps_holes() {
    use crate::core::memory::flat::{Allocation, Flat};
    use crate::core::memory::nest::{ActiveRange, Nest, SpaceKind};

    const SPAN: usize = 3 * PAGE_SIZE;
    let held_before = frame_occupied() - block_occupied();
    let nest = Nest::new(SpaceKind::User).expect("nest: bare address space");
    let flat = Flat::acquire(Allocation::new(3, PAGE_SIZE).expect("nest: allocation"))
        .expect("nest: flat");
    let source = flat.whole();
    source.materialize().expect("nest: materialize the source");
    assert_eq!(source.materialized_frames().expect("nest: frames").len(), 3);
    let at = VirtAddr::wrap(BASE);
    let reservation = nest
        .reserve(Placement::exact(RegionId::Normal, at, SPAN).expect("nest: exact"))
        .expect("nest: reserve the span");
    let active = [
        ActiveRange::new(at, PAGE_SIZE, rw(), rw()).expect("nest: first fragment"),
        ActiveRange::new(at + 2 * PAGE_SIZE, PAGE_SIZE, rw(), rw()).expect("nest: third fragment"),
    ];
    let view = nest
        .attach_shaped(
            reservation,
            source,
            PteFlags::A | PteFlags::D | PteFlags::U,
            &active,
        )
        .expect("nest: shaped attach");
    assert_eq!(view.active().expect("nest: active").len(), 2);
    assert_eq!(view.initial_range(), (at, SPAN));
    assert_eq!(view.frame_count(), 2);
    assert!(nest.translate(at).is_some());
    assert!(
        nest.translate(at + PAGE_SIZE).is_none(),
        "the hole stays unpublished"
    );
    assert!(nest.translate(at + 2 * PAGE_SIZE).is_some());
    nest.audit();
    nest.detach(view.whole()).expect("nest: detach the shape");
    assert!(!view.is_live());
    assert!(nest.translate(at).is_none());
    drop(view);
    drop(nest);
    drop(flat);
    assert_eq!(frame_occupied() - block_occupied(), held_before);
}

/// 部分撤下只交还那一段；同一身份再撤已结算的页失败；整段只碰还欠着的段。
fn partial_detach_returns_only_that_span() {
    const SPAN: usize = 3 * PAGE_SIZE;
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let middle = Frame::allocate().expect("nest: middle frame");
    let weak = Arc::downgrade(&middle);
    let tables = nest.table_count();
    let frames = vec![
        Frame::allocate().expect("nest: frame a"),
        middle,
        Frame::allocate().expect("nest: frame c"),
    ];
    let view = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), SPAN)
                .expect("nest: reservation"),
            frames,
            rw(),
            rw(),
        )
        .expect("nest: map three pages");
    let middle = select(view.view(), VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE);
    nest.detach(middle.clone()).expect("nest: partial detach");
    assert_eq!(view.view().active().expect("nest: active").len(), 2);
    assert!(nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    assert_eq!(
        nest.materialize_active(middle.clone()).err(),
        Some(NestError::NoRegion)
    );
    nest.materialize_active(view.view().whole())
        .expect("nest: materialize only the remaining active pages");
    assert!(nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    // 同叶表里还有邻页，部分撤下不动表页；整段撤完才回到初始页表数。
    assert!(nest.table_count() >= 1);
    // 交还的那一段可以被新关系重新登记并挂载。
    let reused = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(
                    RegionId::Normal,
                    VirtAddr::wrap(BASE + PAGE_SIZE),
                    PAGE_SIZE,
                )
                .expect("nest: the detached span is reusable"),
            vec![Frame::allocate().expect("nest: reused frame")],
            rw(),
            rw(),
        )
        .expect("nest: reuse the span");
    assert!(nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_some());
    assert!(Arc::ptr_eq(
        &nest
            .probe(VirtAddr::wrap(BASE + PAGE_SIZE))
            .expect("nest: reused view"),
        reused.view()
    ));
    // 同一选择再撤一次：这段已经结算，不再欠它。
    assert_eq!(
        nest.detach(middle)
            .err()
            .expect("nest: stale page selection"),
        NestError::NoRegion
    );
    // 整段只处理还欠着的两段，不会去补中间那个洞。
    space
        .memory
        .unmap(&reused)
        .expect("nest: release the reused span");
    nest.detach(view.view().whole())
        .expect("nest: whole after partial");
    assert!(!view.view().is_live());
    assert!(nest.probe(VirtAddr::wrap(BASE)).is_none());
    assert_eq!(nest.table_count(), tables);
    nest.audit();
    // 撤销只撤 PTE 与占用：中间那帧仍由来源强持。
    assert!(weak.upgrade().is_some());
    drop(view);
    drop(space);
    assert!(weak.upgrade().is_none());
}

fn reserve_installs_no_pte() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let at = VirtAddr::wrap(BASE);
    let tables = nest.table_count();
    let reservation = space
        .memory
        .reserve(RegionId::Normal, at, PAGE_SIZE)
        .expect("nest: reserve");
    // 登记只写占用表：页表没有变，也没有翻译。
    assert_eq!(nest.table_count(), tables);
    assert!(nest.probe(at).is_none());
    assert!(nest.translate(at).is_none());
    assert_eq!(reservation.start(), at);
    assert_eq!(reservation.size(), PAGE_SIZE);
    // 同一段再登记必须撞上全局重叠检查。
    assert_eq!(
        space
            .memory
            .reserve(RegionId::Normal, at, PAGE_SIZE)
            .err()
            .expect("nest: overlap rejected"),
        NestError::NoRegion
    );
    space
        .memory
        .release_reservation(reservation)
        .expect("nest: release");
    let again = space
        .memory
        .reserve(RegionId::Normal, at, PAGE_SIZE)
        .expect("nest: reuse after release");
    space
        .memory
        .release_reservation(again)
        .expect("nest: release again");
}

fn guard_split_keeps_both_spans() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let room = space
        .memory
        .allocate_high(RegionId::Normal, 3 * PAGE_SIZE)
        .expect("nest: stack room");
    let (guard, body) = space
        .memory
        .split_reservation(room, PAGE_SIZE)
        .expect("nest: split guard");
    assert_eq!(guard.size(), PAGE_SIZE);
    assert_eq!(body.size(), 2 * PAGE_SIZE);
    assert_eq!(
        body.start().as_usize(),
        guard.start().as_usize() + PAGE_SIZE
    );
    // 守卫页自己占一段：主体和守卫都不会把对方的跨度让出去。
    assert!(
        space
            .memory
            .reserve(RegionId::Normal, guard.start(), PAGE_SIZE)
            .is_err()
    );
    assert!(
        space
            .memory
            .reserve(RegionId::Normal, body.start(), body.size())
            .is_err()
    );
    // 交还守卫不影响主体。
    let guard_start = guard.start();
    space
        .memory
        .release_reservation(guard)
        .expect("nest: release guard");
    let probe = space
        .memory
        .reserve(RegionId::Normal, guard_start, PAGE_SIZE)
        .expect("nest: guard span is reusable on its own");
    space
        .memory
        .release_reservation(probe)
        .expect("nest: release probe");
    assert!(
        space
            .memory
            .reserve(RegionId::Normal, body.start(), body.size())
            .is_err()
    );
    space
        .memory
        .release_reservation(body)
        .expect("nest: release body");
}

fn top_page_exact_and_split() {
    use crate::core::memory::flat::Flat;
    use crate::core::memory::nest::{ActiveRange, Nest, SpaceKind};

    const TWO_PAGES: usize = 2 * PAGE_SIZE;
    let nest = Nest::new(SpaceKind::User).expect("nest: bare address space");
    let head = VirtAddr::wrap(usize::MAX - TWO_PAGES + 1);
    let tail = crate::machine::layout::TRAMPOLINE;
    assert_eq!(usize::MAX - PAGE_SIZE + 1, tail.as_usize());
    let span = nest
        .reserve(Placement::exact(RegionId::Static, head, TWO_PAGES).expect("nest: top placement"))
        .expect("nest: top span");
    assert_eq!(span.start().as_usize() + (span.size() - 1), usize::MAX);
    // 跨窗口的 Exact 与窗口内选择共用同一份全局重叠检查。
    assert!(
        nest.reserve(Placement::exact(RegionId::Normal, tail, PAGE_SIZE).expect("nest: exact"))
            .is_err()
    );
    let (left, right) = nest
        .split_reservation(span, PAGE_SIZE)
        .expect("nest: split the top span");
    assert_eq!(right.start(), tail);
    assert_eq!(right.start().as_usize() + (right.size() - 1), usize::MAX);

    let frame = Frame::allocate().expect("nest: top frame");
    let weak = Arc::downgrade(&frame);
    let region =
        unsafe { BorrowedRegion::new(PhysAddr::from_raw(frame.physical_address()), PAGE_SIZE) }
            .expect("nest: borrowed top page");
    let source = Flat::from_region(region)
        .expect("nest: borrowed source")
        .whole();
    let active = [ActiveRange::new(tail, PAGE_SIZE, rw(), rw()).expect("nest: top active")];
    let top = nest
        .attach_shaped(right, source, PteFlags::A | PteFlags::D, &active)
        .expect("nest: attach the top page");
    assert!(top.is_live());
    assert_eq!(nest.leaf_span(tail), Some(PAGE_SIZE));
    assert_eq!(
        nest.translate(VirtAddr::wrap(usize::MAX))
            .expect("nest: last byte")
            .0,
        PhysAddr::from_raw(frame.physical_address() + PAGE_SIZE - 1)
    );
    // 改权不动架构属性。
    nest.set_access(select(&top, tail, PAGE_SIZE), Access::READ)
        .expect("nest: narrow the current access");
    assert_eq!(top.active().expect("nest: active")[0].current, Access::READ);
    let (_, flags) = nest.translate(tail).expect("nest: top translation");
    assert!(flags.contains(PteFlags::R | PteFlags::A | PteFlags::D));
    assert!(!flags.contains(PteFlags::W));
    // 撕下顶页并回收它的表。
    let tables = nest.table_count();
    nest.detach(select(&top, tail, PAGE_SIZE))
        .expect("nest: detach the top page");
    assert!(nest.translate(tail).is_none());
    assert!(nest.table_count() < tables);
    assert!(weak.upgrade().is_some());
    drop(top);
    nest.release_reservation(left)
        .expect("nest: release the head span");
    let again = nest
        .reserve(Placement::exact(RegionId::Static, tail, PAGE_SIZE).expect("nest: exact"))
        .expect("nest: top reuse after release");
    nest.release_reservation(again)
        .expect("nest: release the top span");
    drop(nest);
    drop(frame);
    assert!(weak.upgrade().is_none());
}

fn detached_selection_and_closed_space() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let at = VirtAddr::wrap(BASE);
    let frame = Frame::allocate().expect("nest: frame");
    let view = frame_view(&space, BASE, frame);
    let page = select(&view.view(), at, PAGE_SIZE);
    nest.detach(page.clone()).expect("nest: detach the page");
    // 已撤下的选择不再成立：不半撤、也不重放。
    assert_eq!(
        nest.detach(page)
            .err()
            .expect("nest: stale selection rejected"),
        NestError::NoRegion
    );
    assert!(view.view().active().expect("nest: active").is_empty());
    // 整笔关系的重试路径交还占用（这里已经没有活动区间）。
    space.memory.unmap(&view).expect("nest: whole retry");
    drop(view);
    let free = nest
        .reserve(Placement::exact(RegionId::Normal, at, PAGE_SIZE).expect("nest: exact"))
        .expect("nest: released span is reusable");
    space
        .memory
        .release_reservation(free)
        .expect("nest: release");
    nest.close().expect("nest: close the space");
    // 关闭之后不再接受新挂载。
    let reservation = nest
        .reserve(Placement::exact(RegionId::Normal, at, PAGE_SIZE).expect("nest: exact"))
        .expect("nest: reserve after close");
    let frame = Frame::allocate().expect("nest: frame");
    assert_eq!(
        space
            .memory
            .map_frames(reservation, vec![frame], rw(), rw())
            .err()
            .expect("nest: closed space rejects new views"),
        NestError::NoRegion
    );
}

fn access_failure_keeps_the_relation_open() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let at = VirtAddr::wrap(BASE);
    let frame = Frame::allocate().expect("nest: frame");
    let view = frame_view(&space, BASE, frame);
    identity::fail_next_shootdown(space.memory.asid());
    assert_eq!(
        nest.set_access(select(view.view(), at, PAGE_SIZE), Access::READ)
            .err()
            .expect("nest: access sync fails"),
        NestError::Shootdown
    );
    // 权限已经改了、活动关系还在，也没有进入 Closing。
    assert!(nest.has_pending_invalidation());
    assert_eq!(
        view.view().active().expect("nest: active")[0].current,
        Access::READ
    );
    assert!(view.view().is_live());
    nest.retry_invalidation().expect("nest: settle access");
    let (_, flags) = nest.translate(at).expect("nest: translation");
    assert!(flags.contains(PteFlags::R | PteFlags::V));
    assert!(!flags.contains(PteFlags::W));
    space.memory.unmap(&view).expect("nest: release");
}

fn kernel_space_is_pinned() {
    let team = crate::core::work::unit::team::kernel().expect("kernel team");
    assert!(team.space.memory.asid().is_kernel());
    assert_eq!(
        team.space
            .memory
            .backend()
            .close()
            .err()
            .expect("kernel space is pinned"),
        NestError::Pinned
    );
}

fn large_leaf_splits_on_access_change() {
    // 留出一段不属于 RAM 的借用区间：用例只核对 PTE，不碰数据。
    const PA: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let at = VirtAddr::wrap(BASE);
    let region = unsafe { BorrowedRegion::new(PhysAddr::from_raw(PA), MEGA) }
        .expect("nest: reserved interval");
    let reservation = nest
        .reserve(Placement::exact(RegionId::Normal, at, MEGA).expect("nest: large placement"))
        .expect("nest: reserve the large span");
    let view = space
        .memory
        .attach_static(
            reservation,
            region,
            0,
            MEGA,
            rw(),
            PteFlags::A | PteFlags::D | PteFlags::U,
        )
        .expect("nest: attach the large span");
    assert_eq!(nest.leaf_span(at), Some(MEGA));
    assert_eq!(
        nest.translate(at + MEGA - 1)
            .expect("nest: large page tail")
            .0,
        PhysAddr::from_raw(PA + MEGA - 1)
    );
    nest.set_access(select(view.view(), at + PAGE_SIZE, PAGE_SIZE), Access::READ)
        .expect("nest: split the large leaf");
    assert_eq!(nest.leaf_span(at + PAGE_SIZE), Some(PAGE_SIZE));
    assert_eq!(
        nest.translate(at + PAGE_SIZE)
            .expect("nest: narrowed page")
            .0,
        PhysAddr::from_raw(PA + PAGE_SIZE)
    );
    assert!(
        !nest
            .translate(at + PAGE_SIZE)
            .expect("nest: narrowed page")
            .1
            .contains(PteFlags::W)
    );
    assert!(
        nest.translate(at + 2 * PAGE_SIZE)
            .expect("nest: sibling page")
            .1
            .contains(PteFlags::W)
    );
    space
        .memory
        .unmap(&view)
        .expect("nest: release the large span");
}

fn noncontiguous_frames_translate_per_page() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let mut owned = vec![
        Frame::allocate().expect("nest: frame a"),
        Frame::allocate().expect("nest: frame b"),
        Frame::allocate().expect("nest: frame c"),
    ];
    owned.sort_by_key(|frame| frame.physical_address());
    let high = owned.pop().expect("nest: high frame");
    let low = owned.remove(0);
    let middle = owned.pop().expect("nest: middle frame");
    assert!(owned.is_empty());
    let middle_held = Arc::strong_count(&middle);
    assert!(high.physical_address() - low.physical_address() >= 2 * PAGE_SIZE);
    unsafe { ::core::ptr::write_volatile(low.as_mut_ptr(), 0x5a) };
    let view = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), 2 * PAGE_SIZE)
                .expect("nest: reservation"),
            vec![low.clone(), high.clone()],
            rw(),
            rw(),
        )
        .expect("nest: map two frames");
    assert_eq!(
        nest.translate(VirtAddr::wrap(BASE))
            .expect("nest: first page")
            .0,
        PhysAddr::from_raw(low.physical_address())
    );
    assert_eq!(
        nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE))
            .expect("nest: second page")
            .0,
        PhysAddr::from_raw(high.physical_address())
    );
    let listed = view
        .view()
        .frames_in(VirtAddr::wrap(BASE), 2 * PAGE_SIZE)
        .expect("nest: published frames");
    assert_eq!(listed.len(), 2);
    assert!(Arc::ptr_eq(&listed[0].1, &low));
    assert!(Arc::ptr_eq(&listed[1].1, &high));
    assert!(!listed.iter().any(|(_, frame)| Arc::ptr_eq(frame, &middle)));
    // 内容随帧走：写进去的字节还能从帧本身读回。
    assert_eq!(unsafe { ::core::ptr::read_volatile(low.as_ptr()) }, 0x5a);
    drop(listed);
    // 两页物理不连续：第二页不接在第一页后面。
    assert_ne!(
        nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE))
            .expect("nest: second page")
            .0,
        PhysAddr::from_raw(low.physical_address() + PAGE_SIZE)
    );
    // 强持账：两帧各有 测试句柄 + Flat 来源槽 + 本关系发布记录 三份。
    assert_eq!(
        Arc::strong_count(&low),
        3,
        "low: {}",
        Arc::strong_count(&low)
    );
    assert_eq!(Arc::strong_count(&high), 3);
    assert_eq!(Arc::strong_count(&middle), middle_held);
    nest.audit();
    space.memory.unmap(&view).expect("nest: release");
    // 撤销只撤 PTE 与发布记录：来源槽仍在，直到关系本身被丢弃。
    assert_eq!(Arc::strong_count(&low), 2);
    assert_eq!(nest.view_count(), 1);
    space.memory.backend().audit();
    drop(view);
    drop(space);
    assert_eq!(Arc::strong_count(&low), 1);
    assert_eq!(Arc::strong_count(&high), 1);
    assert_eq!(Arc::strong_count(&middle), middle_held);
    drop(low);
    drop(high);
    drop(middle);
}

fn lazy_page_is_published_one_at_a_time() {
    let space = SpaceBuilder::user().build().expect("nest: user space");
    let nest = space.memory.backend();
    let view = space
        .memory
        .map_lazy(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), 2 * PAGE_SIZE)
                .expect("nest: reservation"),
            rw(),
            rw(),
        )
        .expect("nest: lazy map");
    assert!(nest.translate(VirtAddr::wrap(BASE)).is_none());
    assert!(nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    nest.materialize_active(select(view.view(), VirtAddr::wrap(BASE), PAGE_SIZE))
        .expect("nest: materialize one page");
    assert!(nest.translate(VirtAddr::wrap(BASE)).is_some());
    // 只物化一页：邻页仍是缺口。
    assert!(nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    assert_eq!(view.view().frame_count(), 1);
    // 强持账：这一页此刻由 Flat 来源槽与本关系发布记录各持一份。
    let listed = view
        .view()
        .frames_in(VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("nest: published page");
    assert_eq!(listed.len(), 1);
    // 强持账：快照 + Flat 来源槽 + 本关系发布记录。
    assert_eq!(Arc::strong_count(&listed[0].1), 3);
    let weak = Arc::downgrade(&listed[0].1);
    drop(listed);
    space.memory.unmap(&view).expect("nest: release");
    // 撤销只撤 PTE 与发布记录：来源槽仍在，直到关系本身被丢弃。
    assert_eq!(weak.strong_count(), 1);
    drop(view);
    assert_eq!(weak.strong_count(), 0);
    assert!(weak.upgrade().is_none());
}
