#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr, VirtAddr};
use crate::core::memory::flat::allocator::statistics;
use crate::core::memory::flat::allocator::statistics::Kind;
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatError, FlatRange, Frame};
use crate::core::memory::nest::{Access, NestError, RegionId, View, ViewRange, identity};
use crate::core::work::unit::space::{Space, SpaceBuilder};

const BASE: usize = 0x6000_0000;

/// 一段页对齐窗口在某个关系上的身份选择。
fn select(view: &Arc<View>, start: usize, size: usize) -> ViewRange {
    let offset = (start - view.initial_start().as_usize()) / PAGE_SIZE;
    view.range(
        PageOffset::new(offset),
        PageCount::new(size / PAGE_SIZE).expect("flat: page count"),
    )
    .expect("flat: selection")
}

/// 一段测试用的用户地址空间。
fn space() -> Space {
    SpaceBuilder::user().build().expect("flat: space")
}

/// 在指定跨度上占一段并挂上既有帧。
fn map_at(space: &Space, start: usize, frames: Vec<Arc<Frame>>) -> crate::core::work::unit::space::Mapping {
    let size = frames.len() * PAGE_SIZE;
    let reservation = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(start), size)
        .expect("flat: reservation");
    space
        .memory
        .map_frames(
            reservation,
            frames,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
        )
        .expect("flat: map frames")
}

/// Flat 来源模型的验收：来源持有、物化唯一、范围合成、借用语义与失败保留。
///
/// 判据都是相对读数：本函数进入前后的帧占用与逐类净额必须为零。
pub fn accept() {
    // 热身：首次建拆一个空间会触发一次性的惰性分配（核身份表与池的首次取自帧池），
    // 那不是来源模型的账；先走一遍，再取基线。
    drop(SpaceBuilder::user().build().expect("flat: warm-up space"));

    // 与既有健康用例同口径：块池从帧池取的整页由 `block_occupied` 单记，
    // 两数之差才是真正的 `Frame` 分配；池会把取过的页缓存下来，不进净账。
    let frames_before = statistics::frame_occupied() - statistics::block_occupied();
    let kinds_before = statistics::kinds();

    let cases: [(&str, fn()); 12] = [
        ("acquire", acquire_is_a_logical_source),
        ("materialize", materialize_resolves_the_same_page),
        ("sub-range", sub_range_keeps_source_identity),
        ("sparse frames", sparse_frames_preserve_content),
        ("borrowed", borrowed_source_is_not_ram),
        (
            "allocation constraints",
            allocation_constraints_are_rejected,
        ),
        (
            "partial materialize",
            partial_materialize_keeps_successful_slots,
        ),
        (
            "map retention",
            map_source_retains_pages_until_the_map_drops,
        ),
        ("derived source", derived_source_outlives_the_source_tree),
        ("partial unmap", partial_unmap_leaves_no_published_hole),
        ("flat alias", one_frame_under_two_indexes),
        ("map alias", aliased_frames_retire_independently),
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
    crate::expect!(off.is_empty(), "flat: per-case net: {:?}", off);

    let frames_after = statistics::frame_occupied() - statistics::block_occupied();
    crate::expect!(
        frames_after == frames_before,
        "flat: frame occupancy {frames_before} → {frames_after} (net must be 0)"
    );
    let kinds_after = statistics::kinds();
    for (kind, count) in kinds_after.nonzero() {
        if kind == Kind::Prime {
            // 块池取自帧池的整页；它们的账在 `block_occupied` 上，不在帧类目里。
            continue;
        }
        crate::expect!(
            count == kinds_before.get(kind),
            "flat: kind {}: {} → {count} (net must be 0)",
            kind.name(),
            kinds_before.get(kind)
        );
    }
}

/// 建立来源不预约 VA、不分配数据页。
fn acquire_is_a_logical_source() {
    let space = SpaceBuilder::user().build().expect("flat: probe space");
    let probe = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("flat: probe reservation");
    space
        .memory
        .release_reservation(probe)
        .expect("flat: probe release");
    let before = statistics::frame_occupied();

    let flat = Flat::acquire(Allocation::new(4, PAGE_SIZE).expect("flat: allocation"))
        .expect("flat: acquire");

    crate::expect!(
        flat.page_count().get() == 4,
        "flat: acquire keeps the declared page count"
    );
    crate::expect!(
        statistics::frame_occupied() == before,
        "flat: acquire must not materialize data pages"
    );
    let whole = flat.whole();
    crate::expect!(
        whole.index().get() == 0,
        "flat: whole starts at the source origin"
    );
    crate::expect!(
        whole.page_count().get() == 4,
        "flat: whole covers the source"
    );
    crate::expect!(
        whole
            .materialized_frames()
            .expect("flat: fresh snapshot")
            .is_empty(),
        "flat: a fresh source has no page slots"
    );
    // 这个 Flat 携带不了 VA：它没有任何 Space / Nest 入口；同一地址桶仍可受理同一段。
    let again = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
        .expect("flat: acquire must not occupy VA");
    space
        .memory
        .release_reservation(again)
        .expect("flat: second release");
    drop(whole);
    drop(flat);
    drop(space);
}

/// 同一逻辑页只有一个物化结果，重复物化拿到同一 Frame。
fn materialize_resolves_the_same_page() {
    let flat = Flat::acquire(Allocation::new(3, PAGE_SIZE).expect("flat: allocation"))
        .expect("flat: acquire");
    let whole = flat.whole();
    let first = whole.materialize().expect("flat: first materialize");
    let second = whole.materialize().expect("flat: second materialize");
    crate::expect!(
        first.page_count().get() == 3,
        "flat: materialized page count"
    );
    for page in 0..3 {
        let offset = PageOffset::new(page);
        crate::expect!(
            first.physical(offset).expect("flat: first physical")
                == second.physical(offset).expect("flat: second physical"),
            "flat: repeated materialize resolves the same physical page"
        );
    }
    let slots = whole
        .materialized_frames()
        .expect("flat: materialized snapshot");
    crate::expect!(
        slots.len() == 3,
        "flat: a materialized window has one slot per page"
    );
    // 同一物理页：写穿一份快照，另一份读数不变，说明持有是共享而不是拷贝。
    let address = first
        .physical(PageOffset::ZERO)
        .expect("flat: first page")
        .as_usize();
    unsafe { ::core::ptr::write_bytes(address as *mut u8, 0x5a, PAGE_SIZE) };
    let via_slot = slots[0].1.physical_address();
    crate::expect!(
        via_slot == address,
        "flat: slot and Pages name the same frame"
    );
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(address as *const u8) } == 0x5a,
        "flat: page content is shared through the source"
    );
    drop(second);
    drop(first);
    drop(whole);
    drop(flat);
}

/// 连续取子范围等价于合成偏移后直接取范围，且保持同一来源身份。
fn sub_range_keeps_source_identity() {
    let flat = Flat::acquire(Allocation::new(4, PAGE_SIZE).expect("flat: allocation"))
        .expect("flat: acquire");
    let whole = flat.whole();
    let pages = whole.materialize().expect("flat: materialize");
    let tail = whole
        .range(PageOffset::new(2), PageCount::new(2).expect("flat: count"))
        .expect("flat: tail selection");
    let single = tail
        .range(PageOffset::new(1), PageCount::new(1).expect("flat: count"))
        .expect("flat: composed selection");
    crate::expect!(
        single.index().get() == 3,
        "flat: composed selection keeps the source offset"
    );
    crate::expect!(
        single.page_count().get() == 1,
        "flat: composed selection keeps its length"
    );
    crate::expect!(
        !single.is_borrowed(),
        "flat: an acquired source is managed RAM"
    );
    let composed = single
        .materialize()
        .expect("flat: composed materialize")
        .physical(PageOffset::ZERO)
        .expect("flat: composed physical");
    let direct = pages
        .physical(PageOffset::new(3))
        .expect("flat: direct physical");
    crate::expect!(
        composed == direct,
        "flat: composition resolves the original page"
    );
    crate::expect!(
        whole
            .range(PageOffset::new(3), PageCount::new(2).expect("flat: count"))
            .is_err(),
        "flat: selection past the source is rejected"
    );
}

/// 接入既有帧：内容与 PA 不变，逻辑页号稀疏，越界/重复/乱序都拒绝。
fn sparse_frames_preserve_content() {
    let low = Frame::allocate().expect("flat: low frame");
    let high = Frame::allocate().expect("flat: high frame");
    let low_address = low.physical_address();
    let high_address = high.physical_address();
    unsafe {
        ::core::ptr::write_bytes(low.as_mut_ptr(), 0xa5, PAGE_SIZE);
        ::core::ptr::write_bytes(high.as_mut_ptr(), 0x3c, PAGE_SIZE);
    }
    let count = PageCount::new(4).expect("flat: count");
    let flat = Flat::from_frames(
        count,
        &[
            (PageIndex::new(1), low.clone()),
            (PageIndex::new(3), high.clone()),
        ],
    )
    .expect("flat: attach existing frames");
    let whole = flat.whole();
    let pages = whole
        .materialize()
        .expect("flat: materialize attached source");
    crate::expect!(
        pages
            .physical(PageOffset::new(1))
            .expect("flat: page 1")
            .as_usize()
            == low_address,
        "flat: an attached page keeps its physical address"
    );
    crate::expect!(
        pages
            .physical(PageOffset::new(3))
            .expect("flat: page 3")
            .as_usize()
            == high_address,
        "flat: a sparse attached page keeps its physical address"
    );
    let fresh_zero = pages
        .physical(PageOffset::ZERO)
        .expect("flat: page 0")
        .as_usize();
    let fresh_two = pages
        .physical(PageOffset::new(2))
        .expect("flat: page 2")
        .as_usize();
    crate::expect!(
        fresh_zero != low_address && fresh_two != high_address,
        "flat: the unpopulated indexes materialize fresh frames"
    );
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(low_address as *const u8) } == 0xa5,
        "flat: attached content is preserved, not cleared"
    );
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(fresh_zero as *const u8) } == 0,
        "flat: materialized RAM still starts zeroed"
    );
    crate::expect!(
        whole.materialized_frames().expect("flat: snapshot").len() == 4,
        "flat: the window ends up dense"
    );
    crate::expect!(
        matches!(
            Flat::from_frames(
                count,
                &[
                    (PageIndex::new(2), low.clone()),
                    (PageIndex::new(1), high.clone()),
                ],
            ),
            Err(FlatError::OutOfRange)
        ),
        "flat: out-of-order indexes are rejected as OutOfRange"
    );
    crate::expect!(
        matches!(
            Flat::from_frames(
                count,
                &[
                    (PageIndex::new(1), low.clone()),
                    (PageIndex::new(1), high.clone()),
                ],
            ),
            Err(FlatError::OutOfRange)
        ),
        "flat: duplicate indexes are rejected as OutOfRange"
    );
    crate::expect!(
        matches!(
            Flat::from_frames(count, &[(PageIndex::new(4), low.clone())]),
            Err(FlatError::OutOfRange)
        ),
        "flat: an index outside the declared page count is rejected"
    );
    // 接入失败不能丢掉调用方交出的帧。
    crate::expect!(
        low.physical_address() == low_address && high.physical_address() == high_address,
        "flat: rejected attachments keep the caller's frames"
    );
}

/// 借用来源不清零、不分配、不按 RAM 回收；PA 是地址信息，不是持有凭证。
fn borrowed_source_is_not_ram() {
    let frame = Frame::allocate().expect("flat: borrowed backing frame");
    let address = PhysAddr::from_raw(frame.physical_address());
    let before = statistics::frame_occupied();
    unsafe { ::core::ptr::write_bytes(frame.as_mut_ptr(), 0x7e, PAGE_SIZE) };

    let region = unsafe { BorrowedRegion::new(address, PAGE_SIZE) }.expect("flat: borrow region");
    crate::expect!(
        region.start() == address && region.size() == PAGE_SIZE,
        "flat: the borrowed region keeps its bounds"
    );
    let flat = Flat::from_region(region.clone()).expect("flat: borrow source");
    let whole = flat.whole();
    crate::expect!(whole.is_borrowed(), "flat: a region source is borrowed");
    let pages = whole.materialize().expect("flat: materialize borrowed");
    crate::expect!(
        pages
            .physical(PageOffset::ZERO)
            .expect("flat: borrowed page")
            == address,
        "flat: a borrowed page keeps its physical address"
    );
    crate::expect!(pages.page_count().get() == 1, "flat: borrowed page count");
    crate::expect!(
        whole
            .materialized_frames()
            .expect("flat: borrowed snapshot")
            .is_empty(),
        "flat: a borrowed source owns no frames"
    );
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(address.as_usize() as *const u8) } == 0x7e,
        "flat: borrowed storage is not zeroed"
    );
    crate::expect!(
        statistics::frame_occupied() == before,
        "flat: a borrowed source allocates nothing"
    );
    drop(pages);
    drop(whole);
    drop(flat);
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(address.as_usize() as *const u8) } == 0x7e,
        "flat: borrowed storage is not returned to the RAM allocator"
    );
    crate::expect!(
        frame.physical_address() == address.as_usize(),
        "flat: the borrowed backing frame is untouched"
    );

    // 设备区间只做地址算术，不接触任何真实寄存器。
    let device = unsafe { BorrowedRegion::new(PhysAddr::from_raw(0x1000_0000), 4 * PAGE_SIZE) }
        .expect("flat: device region");
    let device_flat = Flat::from_region(device).expect("flat: device source");
    let device_pages = device_flat
        .whole()
        .materialize()
        .expect("flat: device pages");
    crate::expect!(
        device_pages
            .physical(PageOffset::new(3))
            .expect("flat: device page")
            .as_usize()
            == 0x1000_0000 + 3 * PAGE_SIZE,
        "flat: device physical addresses are derived from the region"
    );
    crate::expect!(
        statistics::frame_occupied() == before,
        "flat: a device region allocates nothing"
    );
}

/// 不支持的物理约束明确拒绝，不接受后忽略。
fn allocation_constraints_are_rejected() {
    crate::expect!(
        matches!(
            Allocation::new(1, 2 * PAGE_SIZE),
            Err(FlatError::NotAligned)
        ),
        "flat: over-aligned allocation is rejected"
    );
    crate::expect!(
        matches!(Allocation::new(0, PAGE_SIZE), Err(FlatError::NotAligned)),
        "flat: zero pages is rejected"
    );
    let accepted = Allocation::new(2, PAGE_SIZE).expect("flat: page-granular allocation");
    crate::expect!(
        accepted.page_count().get() == 2,
        "flat: the supported allocation keeps its page count"
    );
}

/// 物化中途失败保留已经成功的页槽，失败不改动旧槽；池恢复后能补齐缺口。
fn partial_materialize_keeps_successful_slots() {
    /// 留给本用例自己的簿记分配：池被取尽时，连快照容器都分不出来。
    const HEADROOM: usize = 16;
    // 取尽帧池，再把窗口开成整个池的两倍：物化必然在中途失败，也不可能伪报完成。
    let mut held = drain_frames();
    let drained = held.len();
    crate::expect!(
        drained > HEADROOM * 2,
        "flat: frame pool too small for the partial-materialize probe (drained {drained})"
    );
    let window = drained.checked_mul(2).expect("flat: window pages");
    held.truncate(drained - HEADROOM);

    let flat = Flat::acquire(Allocation::new(window, PAGE_SIZE).expect("flat: allocation"))
        .expect("flat: acquire");
    let whole = flat.whole();
    let outcome = whole.materialize();
    crate::expect!(
        matches!(outcome, Err(FlatError::OutOfMemory)),
        "flat: materialize beyond the physical pool must fail (drained {drained}, outcome {:?})",
        outcome
            .as_ref()
            .map(|pages| pages.page_count().get())
            .map_err(|error| *error)
    );
    drop(outcome);
    held.truncate(held.len() - HEADROOM);
    let placed = slots(&whole);
    crate::expect!(
        !placed.is_empty(),
        "flat: a failed materialize must keep its successful slots"
    );
    crate::expect!(
        placed.len() < window,
        "flat: a failed materialize must not report the window as complete"
    );
    let first = placed[0].1;

    // 再试一次：池仍被持住 ⇒ 仍失败，且已成功的槽一格不动。
    crate::expect!(
        matches!(whole.materialize(), Err(FlatError::OutOfMemory)),
        "flat: a retry while the pool is held still fails"
    );
    held.truncate(held.len() - HEADROOM);
    let again = slots(&whole);
    crate::expect!(
        again.len() >= placed.len(),
        "flat: a failed retry must not lose a successful slot ({} vs {})",
        again.len(),
        placed.len()
    );
    for (index, entry) in placed.iter().enumerate() {
        crate::expect!(
            again[index] == *entry,
            "flat: a failed retry must not replace an already-materialized page"
        );
    }

    // 归还帧池后，同一来源的首页能补齐，且沿用刚才那一页。
    held.clear();
    let prefix = whole
        .range(PageOffset::ZERO, PageCount::new(1).expect("flat: count"))
        .expect("flat: prefix");
    let pages = prefix.materialize().expect("flat: complete the prefix");
    crate::expect!(
        pages
            .physical(PageOffset::ZERO)
            .expect("flat: prefix page")
            .as_usize()
            == first,
        "flat: completing the range keeps the already-materialized page"
    );
    drop(pages);
    drop(prefix);
    drop(again);
    drop(placed);
    drop(whole);
    drop(flat);
}

/// 窗口内已物化槽的 (偏移, 物理地址) 快照。
fn slots(range: &FlatRange) -> Vec<(usize, usize)> {
    let frames = range.materialized_frames().expect("flat: slot snapshot");
    let mut out = Vec::new();
    out.try_reserve_exact(frames.len())
        .expect("flat: slot snapshot capacity");
    out.extend(
        frames
            .iter()
            .map(|(offset, frame)| (offset.get(), frame.physical_address())),
    );
    out
}

/// 关系的稳定状态强持来源：撤销 PTE 不释放页，最后一份持有才回收。
fn map_source_retains_pages_until_the_map_drops() {
    let space = space();
    let frame = Frame::allocate().expect("flat: frame");
    let weak = Arc::downgrade(&frame);
    let mapping = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
                .expect("flat: reservation"),
            vec![frame],
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
        )
        .expect("flat: map frames");

    identity::fail_next_shootdown(space.memory.asid());
    crate::expect!(
        space.memory.unmap(&mapping) == Err(NestError::Shootdown),
        "flat: a failed sync reports Shootdown"
    );
    crate::expect!(
        weak.upgrade().is_some(),
        "flat: failure keeps the physical resource"
    );
    crate::expect!(
        space
            .memory
            .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
            .is_err(),
        "flat: failure keeps the VA from being reused"
    );
    space.memory.unmap(&mapping).expect("flat: retried unmap");

    crate::expect!(
        space
            .memory
            .backend()
            .translate(VirtAddr::wrap(BASE))
            .is_none(),
        "flat: a successful unmap retracts the PTE"
    );
    crate::expect!(
        weak.upgrade().is_some(),
        "flat: the source still holds the page after the PTE is gone"
    );
    drop(mapping);
    drop(space);
    crate::expect!(
        weak.upgrade().is_none(),
        "flat: the last source reference releases the page"
    );
    let recycled = Frame::allocate().expect("flat: allocation after release");
    drop(recycled);
}

/// 派生映射继承来源偏移：源树消失后派生来源仍解析到同一物理页。
fn derived_source_outlives_the_source_tree() {
    let frame = Frame::allocate().expect("flat: frame");
    let address = frame.physical_address();
    let weak = Arc::downgrade(&frame);
    let source = space();
    let source_map = map_at(&source, BASE, vec![frame]);
    let target = space();
    let target_map = target
        .memory
        .view_record(
            source_map.view(),
            0,
            PAGE_SIZE,
            PAGE_SIZE,
            Access::READ,
            Access::READ,
            Some(VirtAddr::from_raw(BASE + 0x20_000)),
        )
        .expect("flat: derived map");

    drop(source_map);
    source.memory.backend().close().expect("flat: close source");
    drop(source);

    crate::expect!(
        weak.upgrade().is_some(),
        "flat: the derived source keeps the frame alive after the source tree is gone"
    );
    crate::expect!(
        target
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + 0x20_000))
            .expect("flat: derived translation")
            .0
            .as_usize()
            == address,
        "flat: derived identity resolves the original physical page"
    );
    drop(target_map);
    target.memory.backend().close().expect("flat: close target");
    drop(target);
    crate::expect!(
        weak.upgrade().is_none(),
        "flat: closing the last holder releases the frame"
    );
}

/// 部分撤销后派生：被撤销的页在来源里仍然活着，但不得被重新发布。
fn partial_unmap_leaves_no_published_hole() {
    let frames = vec![
        Frame::allocate().expect("flat: first frame"),
        Frame::allocate().expect("flat: middle frame"),
        Frame::allocate().expect("flat: last frame"),
    ];
    let addresses: Vec<usize> = frames
        .iter()
        .map(|frame| frame.physical_address())
        .collect();
    let source = space();
    let source_map = map_at(&source, BASE, frames);
    source
        .memory
        .backend()
        .detach(select(source_map.view(), BASE + PAGE_SIZE, PAGE_SIZE))
        .expect("flat: partial unmap");
    crate::expect!(
        source
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
            .is_none(),
        "flat: the revoked page is no longer translated"
    );
    crate::expect!(
        source_map.view().frame_count() == 2,
        "flat: only the revoked page leaves the publication list"
    );

    let target = space();
    let target_map = target
        .memory
        .view_record(
            source_map.view(),
            0,
            3 * PAGE_SIZE,
            PAGE_SIZE,
            Access::READ,
            Access::READ,
            Some(VirtAddr::from_raw(BASE + 0x20_000)),
        )
        .expect("flat: derived map");
    crate::expect!(
        target
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + 0x20_000 + PAGE_SIZE))
            .is_none(),
        "flat: derive must not republish a revoked hole"
    );
    crate::expect!(
        target
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + 0x20_000))
            .expect("flat: derived first page")
            .0
            .as_usize()
            == addresses[0],
        "flat: derive keeps the first live page"
    );
    crate::expect!(
        target
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + 0x20_000 + 2 * PAGE_SIZE))
            .expect("flat: derived third page")
            .0
            .as_usize()
            == addresses[2],
        "flat: derive keeps the last live page"
    );
    crate::expect!(
        target_map.view().frame_count() == 2,
        "flat: the derived map publishes only the live fragments"
    );
}

/// 一个 `Arc<Frame>` 接入同一 `Flat` 的两个逻辑页：同一物理页，只释放一次。
fn one_frame_under_two_indexes() {
    let frame = Frame::allocate().expect("flat: aliased frame");
    let address = frame.physical_address();
    let weak = Arc::downgrade(&frame);
    unsafe { ::core::ptr::write_bytes(frame.as_mut_ptr(), 0x6d, PAGE_SIZE) };
    let count = PageCount::new(3).expect("flat: count");
    let flat = Flat::from_frames(
        count,
        &[
            (PageIndex::new(0), frame.clone()),
            (PageIndex::new(2), frame.clone()),
        ],
    )
    .expect("flat: attach one frame under two indexes");
    let whole = flat.whole();
    let pages = whole.materialize().expect("flat: materialize aliases");
    let aliased = pages.physical(PageOffset::ZERO).expect("flat: first alias");
    crate::expect!(
        pages
            .physical(PageOffset::new(2))
            .expect("flat: second alias")
            == aliased,
        "flat: both indexes resolve to the same physical page"
    );
    crate::expect!(
        aliased.as_usize() == address,
        "flat: the alias keeps the frame's address"
    );
    crate::expect!(
        pages
            .physical(PageOffset::new(1))
            .expect("flat: middle page")
            .as_usize()
            != address,
        "flat: an index without a slot has its own page"
    );
    unsafe { ::core::ptr::write_volatile(aliased.as_usize() as *mut u8, 0x5b) };
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(address as *const u8) } == 0x5b,
        "flat: an aliased write is visible through the frame"
    );
    let clone = whole.clone();
    let tail = whole
        .range(PageOffset::new(2), PageCount::new(1).expect("flat: count"))
        .expect("flat: aliased tail");
    crate::expect!(
        tail.materialize()
            .expect("flat: materialize the alias tail")
            .physical(PageOffset::ZERO)
            .expect("flat: tail page")
            == aliased,
        "flat: a sub-range of the alias resolves the same page"
    );
    crate::expect!(
        weak.upgrade().is_some(),
        "flat: the source holds the aliased frame"
    );
    drop(tail);
    drop(clone);
    drop(pages);
    drop(whole);
    drop(flat);
    drop(frame);
    crate::expect!(
        weak.upgrade().is_none(),
        "flat: the aliased frame is released once, after its last holder"
    );
}

/// 同一帧的两个逻辑页各自发布 PTE：可独立撤销，派生共享同一物理页。
fn aliased_frames_retire_independently() {
    const TARGET: usize = 0x6100_0000;
    let frame = Frame::allocate().expect("flat: aliased frame");
    let address = frame.physical_address();
    let weak = Arc::downgrade(&frame);
    unsafe { ::core::ptr::write_bytes(frame.as_mut_ptr(), 0x6d, PAGE_SIZE) };
    let source = space();
    let source_map = map_at(&source, BASE, vec![frame.clone(), frame]);

    let first = source
        .memory
        .backend()
        .translate(VirtAddr::from_raw(BASE))
        .expect("flat: first alias")
        .0;
    let second = source
        .memory
        .backend()
        .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
        .expect("flat: second alias")
        .0;
    crate::expect!(
        first == second && first.as_usize() == address,
        "flat: two logical pages share one physical page"
    );
    crate::expect!(
        unsafe { ::core::ptr::read_volatile(address as *const u8) } == 0x6d,
        "flat: aliased content is preserved"
    );

    let target = space();
    let target_map = target
        .memory
        .view_record(
            source_map.view(),
            0,
            2 * PAGE_SIZE,
            PAGE_SIZE,
            Access::READ,
            Access::READ,
            Some(VirtAddr::from_raw(TARGET)),
        )
        .expect("flat: derived aliases");
    crate::expect!(
        target
            .memory
            .backend()
            .translate(VirtAddr::from_raw(TARGET))
            .expect("flat: derived first alias")
            .0
            == first
            && target
                .memory
                .backend()
                .translate(VirtAddr::from_raw(TARGET + PAGE_SIZE))
                .expect("flat: derived second alias")
                .0
                == second,
        "flat: the derived map shares both aliases"
    );

    source
        .memory
        .backend()
        .detach(select(source_map.view(), BASE, PAGE_SIZE))
        .expect("flat: revoke the first alias");
    crate::expect!(
        source
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE))
            .is_none(),
        "flat: the revoked alias PTE is gone"
    );
    crate::expect!(
        source
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
            .is_some(),
        "flat: the sibling alias PTE stays"
    );
    crate::expect!(
        target
            .memory
            .backend()
            .translate(VirtAddr::from_raw(TARGET))
            .is_some(),
        "flat: the derived alias stays"
    );
    crate::expect!(
        weak.upgrade().is_some(),
        "flat: the source still holds the aliased frame"
    );

    target
        .memory
        .backend()
        .detach(select(target_map.view(), TARGET, 2 * PAGE_SIZE))
        .expect("flat: revoke the derived aliases");
    source
        .memory
        .backend()
        .detach(select(source_map.view(), BASE + PAGE_SIZE, PAGE_SIZE))
        .expect("flat: revoke the sibling alias");
    crate::expect!(
        source
            .memory
            .backend()
            .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
            .is_none(),
        "flat: both alias PTEs are gone"
    );
    crate::expect!(
        weak.upgrade().is_some(),
        "flat: the old map still holds the source"
    );

    source
        .memory
        .backend()
        .close()
        .expect("flat: close the alias source");
    target
        .memory
        .backend()
        .close()
        .expect("flat: close the alias target");
    drop(source_map);
    drop(target_map);
    drop(source);
    drop(target);
    crate::expect!(
        weak.upgrade().is_none(),
        "flat: the aliased frame is released once, after the last source holder"
    );
}

/// 取尽帧池并全部持住；调用方负责归还。
///
/// 容器按块大小的整数倍扩容，装台与归还都不经过帧池，故不会把自己的簿记算进耗尽。
fn drain_frames() -> Vec<Arc<Frame>> {
    const CHUNK: usize = 64;
    let mut held = Vec::new();
    loop {
        if held.len() == held.capacity() && held.try_reserve_exact(CHUNK).is_err() {
            break;
        }
        match Frame::allocate() {
            Ok(frame) => held.push(frame),
            Err(_) => break,
        }
    }
    held
}
