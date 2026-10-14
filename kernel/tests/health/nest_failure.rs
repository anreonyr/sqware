use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::identity::fail_next_shootdown;
use crate::core::memory::nest::table::TABLE_LIVE;
use crate::core::memory::nest::{Access, NestError, RegionId};
use crate::core::work::unit::space::Mapping;
use crate::core::work::unit::space::SpaceBuilder;

/// 部分撤下的同步失败：只有那一段的票留着，VA 禁复用；同身份重试才交还。
fn partial_detach_failure_retries_by_identity() {
    use crate::core::memory::address::{PageCount, PageOffset};
    use crate::core::memory::nest::ViewRange;

    const BASE: usize = 0x6400_0000;
    const SPAN: usize = 3 * PAGE_SIZE;
    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user()
        .build()
        .expect("nest_failure: partial space");
    let nest = space.memory.backend();
    let tables = nest.table_count();
    let mut frames = vec![
        Frame::allocate().unwrap(),
        Frame::allocate().unwrap(),
        Frame::allocate().unwrap(),
    ];
    frames.sort_by_key(|frame| frame.physical_address());
    let middle = Arc::downgrade(&frames[1]);
    let view = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::from_raw(BASE), SPAN)
                .expect("nest_failure: partial reservation"),
            frames,
            rw,
            rw,
        )
        .expect("nest_failure: partial view");
    let page = |start: usize, size: usize| -> ViewRange {
        let offset = (start - view.view().initial_start().as_usize()) / PAGE_SIZE;
        view.view()
            .range(
                PageOffset::new(offset),
                PageCount::new(size / PAGE_SIZE).expect("nest_failure: page count"),
            )
            .expect("nest_failure: selection")
    };
    fail_next_shootdown(nest.asid());
    assert_eq!(
        nest.detach(page(BASE + PAGE_SIZE, PAGE_SIZE))
            .err()
            .expect("nest_failure: partial sync fails"),
        NestError::Shootdown
    );
    // 那一段已从活动库存里撤下，但退休票与帧都留着：VA 禁复用。
    assert_eq!(view.view().active().expect("nest_failure: active").len(), 2);
    assert_eq!(
        space
            .memory
            .reserve(
                RegionId::Normal,
                VirtAddr::from_raw(BASE + PAGE_SIZE),
                PAGE_SIZE
            )
            .err()
            .unwrap(),
        NestError::NoRegion,
    );
    assert!(middle.upgrade().is_some());
    // 同身份重试：票结算，那一段才交还。
    nest.retry_invalidation().unwrap();
    assert!(middle.upgrade().is_some());
    let again = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(
                    RegionId::Normal,
                    VirtAddr::from_raw(BASE + PAGE_SIZE),
                    PAGE_SIZE,
                )
                .expect("nest_failure: the retired span comes back"),
            vec![middle.upgrade().expect("nest_failure: middle frame")],
            rw,
            rw,
        )
        .expect("nest_failure: refill the middle");
    space
        .memory
        .unmap(&again)
        .expect("nest_failure: release the refill");
    drop(again);
    // 整个关系收尾之后，页表与帧都回到初始账。
    nest.detach(view.view().whole())
        .expect("nest_failure: whole close after partial");
    assert_eq!(nest.table_count(), tables);
    assert!(nest.probe(VirtAddr::from_raw(BASE)).is_none());
    drop(view);
    drop(space);
    assert!(middle.upgrade().is_none());
}

fn preparation_failure_preserves_the_live_span() {
    use crate::core::memory::address::{PageCount, PageOffset};
    use crate::core::memory::nest::fail_withdraw_prepare;

    const BASE: usize = 0x6500_0000;
    const SPAN: usize = 3 * PAGE_SIZE;
    for whole in [false, true] {
        let before = TABLE_LIVE.load(Ordering::Relaxed);
        let space = SpaceBuilder::user().build().unwrap();
        let nest = space.memory.backend();
        let frames = vec![
            Frame::allocate().unwrap(),
            Frame::allocate().unwrap(),
            Frame::allocate().unwrap(),
        ];
        let physical: Vec<_> = frames
            .iter()
            .map(|frame| frame.physical_address())
            .collect();
        let lifetimes: Vec<_> = frames.iter().map(Arc::downgrade).collect();
        let view = space
            .memory
            .map_frames(
                space
                    .memory
                    .reserve(RegionId::Normal, VirtAddr::wrap(BASE), SPAN)
                    .unwrap(),
                frames,
                Access::READ | Access::WRITE,
                Access::READ | Access::WRITE,
            )
            .unwrap();
        let selection = if whole {
            view.view().whole()
        } else {
            view.view()
                .range(PageOffset::new(1), PageCount::new(1).unwrap())
                .unwrap()
        };
        fail_withdraw_prepare();
        assert_eq!(
            nest.detach(selection.clone()).err(),
            Some(NestError::OutOfMemory)
        );
        assert_eq!(view.view().active().unwrap().len(), 1);
        assert_eq!(view.view().frame_count(), 3);
        assert!(!nest.has_pending_invalidation());
        for (index, expected) in physical.iter().enumerate() {
            let (actual, flags) = nest
                .translate(VirtAddr::wrap(BASE + index * PAGE_SIZE))
                .unwrap();
            assert_eq!(actual.as_usize(), *expected);
            assert!(flags.contains(
                crate::core::memory::nest::table::PteFlags::R
                    | crate::core::memory::nest::table::PteFlags::W
            ));
        }
        nest.retry_invalidation().unwrap();
        assert_eq!(
            space
                .memory
                .reserve(
                    RegionId::Normal,
                    VirtAddr::wrap(BASE + PAGE_SIZE),
                    PAGE_SIZE
                )
                .err(),
            Some(NestError::NoRegion)
        );
        nest.detach(selection.clone()).unwrap();
        let (start, size) = if whole {
            (BASE, SPAN)
        } else {
            (BASE + PAGE_SIZE, PAGE_SIZE)
        };
        let reused = space
            .memory
            .reserve(RegionId::Normal, VirtAddr::wrap(start), size)
            .unwrap();
        space.memory.release_reservation(reused).unwrap();
        if !whole {
            nest.detach(view.view().whole()).unwrap();
        }
        drop(selection);
        drop(view);
        drop(space);
        assert!(lifetimes.iter().all(|frame| frame.upgrade().is_none()));
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    }
}

pub fn invalidation() {
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    preparation_failure_preserves_the_live_span();
    partial_detach_failure_retries_by_identity();
    let space = Arc::new(
        SpaceBuilder::user()
            .build()
            .expect("nest_failure: user space"),
    );
    let nest = space.memory.backend().clone();
    let va = VirtAddr::from_raw(0x6200_0000);
    let rw = Access::READ | Access::WRITE;

    // 已发布、同步失败：占用禁复用，回收责任进清理链；重试沿原身份收尾。
    let good = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::from_raw(0x6300_0000), PAGE_SIZE)
                .expect("nest_failure: good reservation"),
            vec![Frame::allocate().unwrap()],
            rw,
            rw,
        )
        .expect("nest_failure: good view");
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    fail_next_shootdown(nest.asid());
    let reservation = space
        .memory
        .reserve(RegionId::Normal, va, PAGE_SIZE)
        .unwrap();
    assert_eq!(
        space
            .memory
            .map_frames(reservation, vec![frame], rw, rw)
            .err()
            .unwrap(),
        NestError::Shootdown,
    );
    assert!(nest.has_pending_invalidation());
    assert!(nest.probe(va).is_none());
    assert!(nest.translate(va).is_none());
    assert!(held.upgrade().is_some());
    let pending = nest.pending_views().unwrap();
    assert_eq!(pending.len(), 1);
    let failed_install = pending[0].clone();
    drop(pending);
    assert_eq!(
        space
            .memory
            .reserve(RegionId::Normal, va, PAGE_SIZE)
            .err()
            .unwrap(),
        NestError::NoRegion,
    );
    // 同一时间另一笔关系正常撤下（同步成功）：不许把失败的那段 VA 一起结清。
    space
        .memory
        .unmap(&good)
        .expect("nest_failure: normal close");
    assert_eq!(
        space
            .memory
            .reserve(RegionId::Normal, va, PAGE_SIZE)
            .err()
            .unwrap(),
        NestError::NoRegion,
        "a normal close must not settle the failed install's VA"
    );
    nest.retry_invalidation().unwrap();
    // 交付失败的关系由清理链收尾：PTE 与占用都收回，来源仍持着帧。
    assert!(nest.probe(va).is_none());
    assert!(!failed_install.is_live());
    assert!(held.upgrade().is_some());
    let reusable = space
        .memory
        .reserve(RegionId::Normal, va, PAGE_SIZE)
        .expect("nest_failure: the VA comes back after the cleanup");
    space
        .memory
        .release_reservation(reusable)
        .expect("nest_failure: release");

    // 撤销的同步失败：已撤 PTE、帧进退休；VA 禁复用，重试才结算。
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let view = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, va, PAGE_SIZE)
                .unwrap(),
            vec![frame],
            rw,
            rw,
        )
        .unwrap();
    let installed_tables = TABLE_LIVE.load(Ordering::Relaxed);
    fail_next_shootdown(nest.asid());
    assert_eq!(
        space
            .memory
            .unmap(&Mapping::new(view.view().clone()))
            .err()
            .unwrap(),
        NestError::Shootdown
    );
    assert!(held.upgrade().is_some());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed_tables);
    assert_eq!(
        space
            .memory
            .reserve(RegionId::Normal, va, PAGE_SIZE)
            .err()
            .unwrap(),
        NestError::NoRegion,
    );
    nest.retry_invalidation().unwrap();
    // 来源级保留：撤销只撤 PTE；旧关系仍强持这一页。
    assert!(held.upgrade().is_some());
    assert!(TABLE_LIVE.load(Ordering::Relaxed) < installed_tables);
    // 原身份重试之后才交还占用。
    space
        .memory
        .unmap(&Mapping::new(view.view().clone()))
        .unwrap();
    let reused = space
        .memory
        .map_lazy(
            space
                .memory
                .reserve(RegionId::Normal, va, PAGE_SIZE)
                .unwrap(),
            rw,
            rw,
        )
        .unwrap();
    space.memory.unmap(&reused).unwrap();

    // 关闭的同步失败：资源保留，重试后才回收；ASID 最后归还。
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let old = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, va, PAGE_SIZE)
                .unwrap(),
            vec![frame],
            rw,
            rw,
        )
        .unwrap();
    let old_asid = nest.asid();
    let installed_tables = TABLE_LIVE.load(Ordering::Relaxed);
    fail_next_shootdown(old_asid);
    assert_eq!(nest.close().err().unwrap(), NestError::Shootdown);
    assert!(nest.has_pending_invalidation());
    assert!(held.upgrade().is_some());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed_tables);
    nest.retry_invalidation().unwrap();
    // 关闭后的资源保留同样在来源上：地址空间不再发布，但旧关系仍持有来源。
    assert!(held.upgrade().is_some());
    assert!(old.view().owner().is_none());
    drop(nest);
    drop(space);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);

    let replacement = SpaceBuilder::user().build().unwrap();
    let fresh = replacement.memory.backend().clone();
    assert_eq!(fresh.asid(), old_asid);
    // 旧选择带旧身份：新地址空间不认它。
    assert_eq!(
        replacement
            .memory
            .unmap(&Mapping::new(old.view().clone()))
            .err()
            .unwrap(),
        NestError::NoRegion
    );
    let current = replacement
        .memory
        .map_lazy(
            replacement
                .memory
                .reserve(RegionId::Normal, va, PAGE_SIZE)
                .unwrap(),
            rw,
            rw,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&fresh.probe(va).unwrap(), current.view()));
    fresh.close().unwrap();
    drop(fresh);
    drop(replacement);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    // 最后一份来源持有消失后，帧才回到分配器。
    drop(old);
    assert!(held.upgrade().is_none());
}
