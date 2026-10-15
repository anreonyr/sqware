use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::identity::fail_next_shootdown;
use crate::core::memory::nest::table::{PteFlags, TABLE_LIVE};
use crate::core::memory::nest::{Access, NestError};
use crate::core::work::unit::space::SpaceBuilder;

use super::support;

/// 部分撤下的同步失败：只有那一段的票留着，VA 禁复用；同身份重试才交还。
fn partial_detach_failure_retries_by_identity() {
    use crate::core::memory::address::{PageCount, PageOffset};
    use crate::core::memory::nest::ViewRange;

    const BASE: usize = 0x6400_0000;
    let rw = Access::READ | Access::WRITE;
    let space = SpaceBuilder::user()
        .build()
        .expect("nest_failure: partial space");
    let nest = &space.nest;
    let tables = nest.table_count();
    let mut frames = vec![
        Frame::allocate().unwrap(),
        Frame::allocate().unwrap(),
        Frame::allocate().unwrap(),
    ];
    frames.sort_by_key(|frame| frame.physical_address());
    let middle = Arc::downgrade(&frames[1]);
    let view = support::map_frames(&space, VirtAddr::from_raw(BASE), frames, rw, rw)
        .expect("nest_failure: partial view");
    let page = |start: usize, size: usize| -> ViewRange {
        let offset = (start - view.initial_start().as_usize()) / PAGE_SIZE;
        view.range(
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
    assert_eq!(view.active().expect("nest_failure: active").len(), 2);
    assert_eq!(
        support::reserve(&space, VirtAddr::from_raw(BASE + PAGE_SIZE), PAGE_SIZE)
            .err()
            .unwrap(),
        NestError::NoRegion,
    );
    assert!(middle.upgrade().is_some());
    // 同身份重试：票结算，那一段才交还。
    nest.retry_invalidation().unwrap();
    assert!(middle.upgrade().is_some());
    let again = support::map_frames(
        &space,
        VirtAddr::from_raw(BASE + PAGE_SIZE),
        vec![middle.upgrade().expect("nest_failure: middle frame")],
        rw,
        rw,
    )
    .expect("nest_failure: refill the middle");
    space
        .nest
        .detach(again.whole())
        .expect("nest_failure: release the refill");
    drop(again);
    // 整个关系收尾之后，页表与帧都回到初始账。
    nest.detach(view.whole())
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
        let nest = &space.nest;
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
        let view = support::map_frames(
            &space,
            VirtAddr::wrap(BASE),
            frames,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
        )
        .unwrap();
        let selection = if whole {
            view.whole()
        } else {
            view.range(PageOffset::new(1), PageCount::new(1).unwrap())
                .unwrap()
        };
        fail_withdraw_prepare();
        assert_eq!(
            nest.detach(selection.clone()).err(),
            Some(NestError::OutOfMemory)
        );
        assert_eq!(view.active().unwrap().len(), 1);
        assert_eq!(view.frame_count(), 3);
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
            support::reserve(&space, VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE).err(),
            Some(NestError::NoRegion)
        );
        nest.detach(selection.clone()).unwrap();
        let (start, size) = if whole {
            (BASE, SPAN)
        } else {
            (BASE + PAGE_SIZE, PAGE_SIZE)
        };
        let reused = support::reserve(&space, VirtAddr::wrap(start), size).unwrap();
        space.nest.release_reservation(reused).unwrap();
        if !whole {
            nest.detach(view.whole()).unwrap();
        }
        drop(selection);
        drop(view);
        drop(space);
        assert!(lifetimes.iter().all(|frame| frame.upgrade().is_none()));
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    }
}

/// 发布记录容量这一批准备失败（尚未写任何 PTE）：占用退回，同址可再登记，
/// 根页表 / 索引 / 待撤 / 帧账都不留半发布。
fn capacity_preparation_failure_returns_the_span() {
    const BASE: usize = 0x6B00_0000;
    const SPAN: usize = 2 * PAGE_SIZE;
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let space = SpaceBuilder::user()
        .build()
        .expect("nest_failure: capacity space");
    let nest = &space.nest;
    let tables = nest.table_count();
    let views = nest.view_count();
    let frame = Frame::allocate().unwrap();
    let physical = frame.physical_address();
    let weak = Arc::downgrade(&frame);
    unsafe { ::core::ptr::write_volatile(frame.as_mut_ptr(), 0x5e) };
    let source = support::frames_source(vec![frame, Frame::allocate().unwrap()])
        .expect("nest_failure: capacity source");
    let pages = source
        .materialize()
        .expect("nest_failure: materialize source");
    drop(pages);

    // 容量准备被注入失败：这时还没有任何 PTE、没有索引接入、也没有待撤关系。
    crate::core::memory::nest::fail_publish_prepare();
    let reservation = support::reserve(&space, VirtAddr::from_raw(BASE), SPAN).unwrap();
    assert_eq!(
        nest.attach(reservation, source.clone(), Access::READ, Access::READ)
            .err()
            .unwrap(),
        NestError::OutOfMemory
    );
    assert!(nest.probe(VirtAddr::from_raw(BASE)).is_none());
    assert!(nest.translate(VirtAddr::from_raw(BASE)).is_none());
    assert!(!nest.has_pending_invalidation());
    assert!(nest.pending_views().unwrap().is_empty());
    assert_eq!(nest.view_count(), views);
    assert_eq!(nest.table_count(), tables);
    // 尚未发布：占用应当退回，而不是“拒复用”。
    let returned = support::reserve(&space, VirtAddr::from_raw(BASE), SPAN)
        .expect("nest_failure: an unpublished failure must return the span");
    assert_eq!(returned.size(), SPAN);
    space.nest.release_reservation(returned).unwrap();
    // 输入来源仍持原来的帧与内容：后面用同一段来源重新挂载，拿到同一物理页。
    assert!(weak.upgrade().is_some());
    assert_eq!(
        unsafe { ::core::ptr::read_volatile(physical as *const u8) },
        0x5e
    );
    let reservation = support::reserve(&space, VirtAddr::from_raw(BASE), SPAN).unwrap();
    let view = nest
        .attach(reservation, source, Access::READ, Access::READ)
        .expect("nest_failure: retry after capacity failure");
    assert_eq!(
        nest.translate(VirtAddr::from_raw(BASE))
            .unwrap()
            .0
            .as_usize(),
        physical
    );
    let republished = view
        .frames_in(VirtAddr::from_raw(BASE), PAGE_SIZE)
        .expect("nest_failure: republished frame");
    assert!(Arc::ptr_eq(
        &republished[0].1,
        &weak.upgrade().expect("nest_failure: frame alive")
    ));
    drop(republished);
    space
        .nest
        .detach(view.whole())
        .expect("nest_failure: release");
    drop(space);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

/// 消费式输入按真实属主回收：foreign 凭证失败后，属主同址可再登记且拿到新 generation，
/// 接收方的原占用不受影响；非法参数与构造失败同样退回属主。
fn foreign_or_invalid_preparation_returns_to_its_owner() {
    const BASE: usize = 0x6C00_0000;
    const OWN: usize = 0x6C01_0000;
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let a = SpaceBuilder::user().build().expect("nest_failure: owner A");
    let b = SpaceBuilder::user().build().expect("nest_failure: owner B");
    let rw = Access::READ | Access::WRITE;
    let active = |at: usize| {
        [
            crate::core::memory::nest::ActiveRange::new(VirtAddr::from_raw(at), PAGE_SIZE, rw, rw)
                .expect("nest_failure: active range"),
        ]
    };

    // A 自己在 BASE 上也有一笔合法占用：foreign 的失败不得动它。
    let owned = support::reserve(&a, VirtAddr::from_raw(BASE), PAGE_SIZE).unwrap();
    let source = support::frames_source(vec![Frame::allocate().unwrap()]).unwrap();

    // ① 普通入口：B 的凭证交给 A ⇒ NoRegion，B 同址可再登记且是新 generation。
    let foreign = support::reserve(&b, VirtAddr::from_raw(BASE), PAGE_SIZE).unwrap();
    let foreign_id = foreign.id();
    assert_eq!(
        a.nest
            .attach(foreign, source.clone(), rw, rw)
            .err()
            .unwrap(),
        NestError::NoRegion
    );
    assert!(support::reserve(&a, VirtAddr::from_raw(BASE), PAGE_SIZE).is_err());
    let fresh = support::reserve(&b, VirtAddr::from_raw(BASE), PAGE_SIZE).unwrap();
    assert_ne!(fresh.id(), foreign_id);
    b.nest.release_reservation(fresh).unwrap();

    // ② 活动形状入口：foreign 凭证一样退回属主，target 原占用不动。
    let foreign = support::reserve(&b, VirtAddr::from_raw(BASE), PAGE_SIZE).unwrap();
    let foreign_id = foreign.id();
    assert_eq!(
        a.nest
            .attach_shaped(
                foreign,
                source.clone(),
                PteFlags::A | PteFlags::D,
                &active(BASE)
            )
            .err()
            .unwrap(),
        NestError::NoRegion
    );
    assert!(support::reserve(&a, VirtAddr::from_raw(BASE), PAGE_SIZE).is_err());
    let again = support::reserve(&b, VirtAddr::from_raw(BASE), PAGE_SIZE).unwrap();
    assert_ne!(again.id(), foreign_id);
    b.nest.release_reservation(again).unwrap();

    // ③ 非架构属性位：自己的凭证也退回自己的表，同址可再登记。
    let reservation = support::reserve(&a, VirtAddr::from_raw(OWN), PAGE_SIZE).unwrap();
    assert_eq!(
        a.nest
            .attach_shaped(reservation, source.clone(), PteFlags::V, &active(OWN))
            .err()
            .unwrap(),
        NestError::NotAligned
    );
    let reused = support::reserve(&a, VirtAddr::from_raw(OWN), PAGE_SIZE).unwrap();
    a.nest.release_reservation(reused).unwrap();

    // ④ 构造失败（来源跨度与占用不等）：同一份输入退回自己的表。
    let reservation = support::reserve(&a, VirtAddr::from_raw(OWN), 2 * PAGE_SIZE).unwrap();
    assert_eq!(
        a.nest.attach(reservation, source, rw, rw).err().unwrap(),
        NestError::NotAligned
    );
    let reused = support::reserve(&a, VirtAddr::from_raw(OWN), 2 * PAGE_SIZE).unwrap();
    a.nest.release_reservation(reused).unwrap();

    a.nest.release_reservation(owned).unwrap();
    drop(a);
    drop(b);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

pub fn invalidation() {
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    preparation_failure_preserves_the_live_span();
    partial_detach_failure_retries_by_identity();
    capacity_preparation_failure_returns_the_span();
    foreign_or_invalid_preparation_returns_to_its_owner();
    let space = Arc::new(
        SpaceBuilder::user()
            .build()
            .expect("nest_failure: user space"),
    );
    let nest = space.nest.clone();
    let va = VirtAddr::from_raw(0x6200_0000);
    let rw = Access::READ | Access::WRITE;

    // 已发布、同步失败：占用禁复用，回收责任进清理链；重试沿原身份收尾。
    let good = support::map_frames(
        &space,
        VirtAddr::from_raw(0x6300_0000),
        vec![Frame::allocate().unwrap()],
        rw,
        rw,
    )
    .expect("nest_failure: good view");
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    fail_next_shootdown(nest.asid());
    let reservation = support::reserve(&space, va, PAGE_SIZE).unwrap();
    assert_eq!(
        support::attach_frames(&space, reservation, vec![frame], rw, rw)
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
        support::reserve(&space, va, PAGE_SIZE).err().unwrap(),
        NestError::NoRegion,
    );
    // 同一时间另一笔关系正常撤下（同步成功）：不许把失败的那段 VA 一起结清。
    space
        .nest
        .detach(good.whole())
        .expect("nest_failure: normal close");
    assert_eq!(
        support::reserve(&space, va, PAGE_SIZE).err().unwrap(),
        NestError::NoRegion,
        "a normal close must not settle the failed install's VA"
    );
    nest.retry_invalidation().unwrap();
    // 交付失败的关系由清理链收尾：PTE 与占用都收回，来源仍持着帧。
    assert!(nest.probe(va).is_none());
    assert!(!failed_install.is_live());
    assert!(held.upgrade().is_some());
    let reusable = support::reserve(&space, va, PAGE_SIZE)
        .expect("nest_failure: the VA comes back after the cleanup");
    space
        .nest
        .release_reservation(reusable)
        .expect("nest_failure: release");

    // 撤销的同步失败：已撤 PTE、帧进退休；VA 禁复用，重试才结算。
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let view = support::map_frames(&space, va, vec![frame], rw, rw).unwrap();
    let installed_tables = TABLE_LIVE.load(Ordering::Relaxed);
    fail_next_shootdown(nest.asid());
    assert_eq!(
        space.nest.detach(view.whole()).err().unwrap(),
        NestError::Shootdown
    );
    assert!(held.upgrade().is_some());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed_tables);
    assert_eq!(
        support::reserve(&space, va, PAGE_SIZE).err().unwrap(),
        NestError::NoRegion,
    );
    nest.retry_invalidation().unwrap();
    // 来源级保留：撤销只撤 PTE；旧关系仍强持这一页。
    assert!(held.upgrade().is_some());
    assert!(TABLE_LIVE.load(Ordering::Relaxed) < installed_tables);
    // 原身份重试之后才交还占用。
    space.nest.detach(view.whole()).unwrap();
    let reused = support::map_lazy(
        &space,
        support::reserve(&space, va, PAGE_SIZE).unwrap(),
        rw,
        rw,
    )
    .unwrap();
    space.nest.detach(reused.whole()).unwrap();

    // 关闭的同步失败：资源保留，重试后才回收；ASID 最后归还。
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let old = support::map_frames(&space, va, vec![frame], rw, rw).unwrap();
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
    assert!(old.owner().is_none());
    drop(nest);
    drop(space);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);

    let replacement = SpaceBuilder::user().build().unwrap();
    let fresh = replacement.nest.clone();
    assert_eq!(fresh.asid(), old_asid);
    // 旧选择带旧身份：新地址空间不认它。
    assert_eq!(
        replacement.nest.detach(old.whole()).err().unwrap(),
        NestError::NoRegion
    );
    let current = support::map_lazy(
        &replacement,
        support::reserve(&replacement, va, PAGE_SIZE).unwrap(),
        rw,
        rw,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&fresh.probe(va).unwrap(), &current));
    fresh.close().unwrap();
    drop(fresh);
    drop(replacement);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    // 最后一份来源持有消失后，帧才回到分配器。
    drop(old);
    assert!(held.upgrade().is_none());
}
