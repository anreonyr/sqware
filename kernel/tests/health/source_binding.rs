//! 控制绑定的**来源侧成员**：一段未结的控制在来源名下就是一条成员。
//!
//! 覆盖：prepared（未提交）没结清时成员仍在、结清只有一条收尾且它把 Registry 项与
//! 成员一起撤、`hand_over` 改的是两边共享的那一份、同一份来源下两个 Space 里 raw
//! 最后析构丢掉 Registry 时留下的未知成员，以及成功路上的净账。

use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;
use alloc::vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::flat::Frame;
use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
use crate::core::memory::nest::identity::{Asid, fail_shootdown, restore_shootdown};
use crate::core::memory::nest::table::TABLE_LIVE;
use crate::core::memory::nest::{Access, NestError};
use crate::core::work::pend;
use crate::core::work::pend::SourceAuthorization;
use crate::core::work::room::messenger;
use crate::core::work::room::scheduler;
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::core::work::unit::task::TaskWeak;
use crate::core::work::unit::team::{self, TeamBuilder};

use super::support;

const FIRST: VirtAddr = VirtAddr::wrap(0x6300_0000);
const SECOND: VirtAddr = VirtAddr::wrap(0x6400_0000);
const THIRD: VirtAddr = VirtAddr::wrap(0x6600_0000);

fn held_pages() -> usize {
    frame_occupied() - block_occupied()
}

fn rw() -> Access {
    Access::READ | Access::WRITE
}

fn space() -> Arc<Space> {
    Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap()
}

/// prepared（还没提交）的一段：来源名下就是那条未结成员；没结清撤不动，
/// 结清后同一条收尾把它与 Registry 项一起撤。可失败的准备也不留半边。
pub fn prepared_member_survives_until_settled() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let space = space();
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let source = support::frames_source(vec![frame]).unwrap();
    let authorization = SourceAuthorization::new(source, rw(), env::TaskId::new(0)).unwrap();
    let reservation = support::reserve(&space, FIRST, PAGE_SIZE).unwrap();
    let id = reservation.id();
    let claim = || {
        space.views.claim(
            id,
            &space,
            env::TaskId::new(0),
            env::ViewControl::SpaceMembers,
            authorization.clone(),
            None,
        )
    };
    claim().unwrap();
    // 在途槽：登记在，但不可用；来源名下也是同一条成员。
    let entry = space.views.registered(id).expect("prepared registration");
    assert!(space.views.locate(id).is_none());
    assert!(entry.view().is_none());
    assert_eq!(authorization.binding_count(), 1);
    assert!(authorization.owns_binding(&entry.binding));
    let binding = entry.binding.clone();
    drop(entry);
    // 重号拒收：可失败的准备不往两表里添任何东西。
    assert_eq!(claim().err(), Some(env::PieFail::Busy));
    assert_eq!(authorization.binding_count(), 1);
    // 没结清：同一条收尾什么都不撤。
    space.retire_view(id);
    assert!(space.views.registered(id).is_some());
    assert_eq!(authorization.binding_count(), 1);
    // 占用票交还即结清：Registry 项与源侧成员一起撤。
    space.nest.release_reservation(reservation).unwrap();
    space.retire_view(id);
    assert!(space.views.registered(id).is_none());
    assert_eq!(authorization.binding_count(), 0);
    space.retire().unwrap();
    drop(authorization);
    drop(space);
    assert!(binding.space().upgrade().is_none());
    assert!(
        held.upgrade().is_none(),
        "settled: the record releases the frame"
    );
    drop(binding);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), frames);
}

/// 正常结清：ready 的一段也是成员，撤图成功后同一条收尾把两边一起撤，
/// 旧 Space 与这份来源的记录都不再被它占着。
pub fn settled_member_leaves_no_space_or_source_behind() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let space = space();
    let held_space = Arc::downgrade(&space);
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let view = support::map_frames(&space, SECOND, vec![frame], rw(), rw()).unwrap();
    let id = view.identity();
    let authorization =
        SourceAuthorization::new(view.source().clone(), rw(), env::TaskId::new(0)).unwrap();
    let record = Arc::downgrade(&authorization);
    space
        .views
        .claim(
            id,
            &space,
            env::TaskId::new(0),
            env::ViewControl::SpaceMembers,
            authorization.clone(),
            None,
        )
        .unwrap();
    space.views.commit(id, view.clone());
    let entry = space.views.locate(id).expect("ready registration");
    assert_eq!(authorization.binding_count(), 1);
    assert!(authorization.owns_binding(&entry.binding));
    let binding = entry.binding.clone();
    drop(entry);

    space.dispose(view.clone()).unwrap();
    assert!(space.nest.settled(id));
    space.retire_view(id);
    assert!(space.views.registered(id).is_none());
    assert_eq!(authorization.binding_count(), 0);

    // 旧 Space 与这份来源的记录都不被这段成员永久占着。
    space.retire().unwrap();
    drop(view);
    drop(authorization);
    drop(space);
    assert!(held_space.upgrade().is_none());
    assert!(binding.space().upgrade().is_none());
    assert!(record.upgrade().is_none());
    assert!(held.upgrade().is_none());
    drop(binding);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), frames);
}

/// `hand_over` 改的是 Registry 项与来源成员共享的那一份绑定：
/// controller 与分支一次改完，两处不会各留一份。
pub fn hand_over_keeps_one_shared_control() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let space = space();
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let view = support::map_frames(&space, THIRD, vec![frame], rw(), rw()).unwrap();
    let id = view.identity();
    let authorization =
        SourceAuthorization::new(view.source().clone(), rw(), env::TaskId::new(0)).unwrap();
    let branch = env::PieToken::mint(9);
    space
        .views
        .claim(
            id,
            &space,
            env::TaskId::new(4),
            env::ViewControl::SpaceMembers,
            authorization.clone(),
            Some(branch),
        )
        .unwrap();
    space.views.commit(id, view.clone());
    let entry = space.views.locate(id).expect("ready registration");
    assert_eq!(
        entry.control.as_ref().unwrap().control(),
        (env::TaskId::new(4), Some(branch))
    );
    assert!(authorization.owns_binding(&entry.binding));
    assert_eq!(authorization.binding_count(), 1);

    space.views.hand_over(id, env::TaskId::new(5));
    let after = space.views.locate(id).expect("ready registration");
    assert_eq!(
        after.control.as_ref().unwrap().control(),
        (env::TaskId::new(5), None)
    );
    assert!(Arc::ptr_eq(&entry.binding, &after.binding));
    assert!(Arc::ptr_eq(
        entry.control.as_ref().unwrap(),
        after.control.as_ref().unwrap()
    ));
    assert!(authorization.owns_binding(&after.binding));
    assert_eq!(authorization.binding_count(), 1);
    let binding = after.binding.clone();
    drop(entry);
    drop(after);

    space.dispose(view).unwrap();
    space.retire_view(id);
    assert_eq!(authorization.binding_count(), 0);
    space.retire().unwrap();
    drop(authorization);
    drop(space);
    assert!(binding.space().upgrade().is_none());
    assert!(held.upgrade().is_none());
    drop(binding);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), frames);
}

/// 同一份来源下两个 Space：raw 那一个最后析构时撤图未确认，Registry 随它消失、
/// 没有 Nest 可推进；另一个后来正常结清。剩下那条成员是**未知义务**，
/// 不因为“也没别的地方记得”而被撤下。
///
/// 与 `nest_exit::last_drop_with_persistent_failure` 同一条边界：原始析构只保证
/// 禁止复用，保留的表与页不是回收；故此处不声称全净回收。
pub fn raw_last_drop_keeps_unknown_member() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let baseline = TABLE_LIVE.load(Ordering::Relaxed);
    let raw = space();
    let raw_asid = raw.nest.asid();
    let raw_nest = Arc::downgrade(&raw.nest);
    let keep = space();
    // 同一份来源的两页：一页装进 raw，另一页装进 keep。
    let frames = vec![Frame::allocate().unwrap(), Frame::allocate().unwrap()];
    let held = Arc::downgrade(&frames[0]);
    let source = support::frames_source(frames).unwrap();
    let authorization =
        SourceAuthorization::new(source.clone(), rw(), env::TaskId::new(0)).unwrap();
    let first = source
        .range(PageOffset::ZERO, PageCount::new(1).unwrap())
        .unwrap();
    let second = source
        .range(PageOffset::new(1), PageCount::new(1).unwrap())
        .unwrap();

    let raw_reservation = support::reserve(&raw, FIRST, PAGE_SIZE).unwrap();
    let raw_id = raw_reservation.id();
    raw.views
        .claim(
            raw_id,
            &raw,
            env::TaskId::new(0),
            env::ViewControl::SpaceMembers,
            authorization.clone(),
            None,
        )
        .unwrap();
    let raw_view = raw.nest.attach(raw_reservation, first, rw(), rw()).unwrap();
    raw.views.commit(raw_id, raw_view.clone());
    let raw_binding = raw.views.registered(raw_id).unwrap().binding;

    let keep_reservation = support::reserve(&keep, SECOND, PAGE_SIZE).unwrap();
    let keep_id = keep_reservation.id();
    keep.views
        .claim(
            keep_id,
            &keep,
            env::TaskId::new(0),
            env::ViewControl::SpaceMembers,
            authorization.clone(),
            None,
        )
        .unwrap();
    let keep_view = keep
        .nest
        .attach(keep_reservation, second, rw(), rw())
        .unwrap();
    keep.views.commit(keep_id, keep_view.clone());
    let keep_binding = keep.views.registered(keep_id).unwrap().binding;
    assert_eq!(authorization.binding_count(), 2);

    // raw 的最后一次析构：撤图确认不了，Registry 连同 Space 一起没了。
    fail_shootdown(raw_asid);
    assert_eq!(raw.nest.dispose(raw_view), Err(NestError::Shootdown));
    drop(raw);
    assert!(raw_nest.upgrade().is_none());
    assert!(
        raw_binding.space().upgrade().is_none(),
        "the target is gone"
    );
    assert!(
        authorization.owns_binding(&raw_binding),
        "nobody settled that member"
    );
    assert_eq!(authorization.binding_count(), 2);
    // 页仍占着（这段来源自己的范围也压着它）：这里只说“没有回收”，不声称逐侧页账。
    assert!(held.upgrade().is_some());
    let installed = TABLE_LIVE.load(Ordering::Relaxed);
    let installed_frames = held_pages();
    assert!(installed > baseline);

    // 原始析构路径只保证禁止复用：没有 Nest 可推进，表与页都不回收。
    restore_shootdown(raw_asid);
    for _ in 0..2 {
        let replacement = SpaceBuilder::user().build().unwrap();
        assert_ne!(replacement.nest.asid(), raw_asid);
        replacement.nest.retry_invalidation().unwrap();
        drop(replacement);
        messenger::bury();
        assert!(raw_nest.upgrade().is_none());
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed);
        assert_eq!(held_pages(), installed_frames);
    }

    // 另一个 Space 正常结清：它自己那条成员撤下，raw 那条**未知**成员照旧。
    keep.dispose(keep_view).unwrap();
    keep.retire_view(keep_id);
    assert!(keep.views.registered(keep_id).is_none());
    assert!(!authorization.owns_binding(&keep_binding));
    assert!(authorization.owns_binding(&raw_binding));
    assert_eq!(authorization.binding_count(), 1);
    keep.retire().unwrap();
    drop(keep);
    assert!(
        TABLE_LIVE.load(Ordering::Relaxed) > baseline,
        "raw's tables are not returned: no Nest is left to push them"
    );
    drop(authorization);
    crate::putln!(
        "source_binding: raw last drop keeps its tables and page; the settled sibling \
         leaves the unknown member behind"
    );
}

pub fn closed_input_does_not_block_pole_cleanup() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let husks = messenger::probe_bookkeeping().1;
    for fail_deallocate in [false, true] {
        let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
            .spawn()
            .unwrap();
        let owner = parent.task().hold().unwrap();
        let child = TeamBuilder::build(
            crate::core::memory::nest::SpaceKind::User,
            TaskWeak::stored(Arc::downgrade(&owner), Origin::Sire),
        )
        .unwrap();
        let frame = Frame::allocate().unwrap();
        let held = Arc::downgrade(&frame);
        let view = support::map_frames(&child.space, FIRST, vec![frame], rw(), rw()).unwrap();
        let id = view.identity();
        let (token, meta) = support::source_pole(&owner, &child.space, &view, true);
        let authorization = meta.authorization().unwrap();
        let asid = child.space.nest.asid();
        assert_eq!(authorization.binding_count(), 1);
        assert!(meta.has_maps());

        fail_shootdown(asid);
        assert_eq!(team::oust(&owner, child.id), Err(env::UnitFail::Busy));
        crate::core::work::mail::pole::seal(&meta);
        assert_eq!(meta.close_maps(None), Err(env::PieFail::Busy));
        assert!(meta.has_maps());
        assert_eq!(authorization.binding_count(), 1);
        assert!(held.upgrade().is_some());

        restore_shootdown(asid);
        child.space.nest.close().unwrap();
        if fail_deallocate {
            crate::core::memory::nest::identity::fail_next_shootdown(asid);
            assert_eq!(team::oust(&owner, child.id), Err(env::UnitFail::Busy));
        } else {
            assert_eq!(team::oust(&owner, child.id), Ok(()));
        }
        assert_eq!(child.space.nest.retired(), !fail_deallocate);
        assert!(child.space.nest.contains_reservation(id));
        assert!(child.space.nest.settled(id));
        assert_eq!(authorization.binding_count(), 0);
        assert!(
            meta.has_maps(),
            "the Pole still owns the already settled input record"
        );
        assert_eq!(
            meta.close_maps(None),
            Ok(()),
            "a retained closed Space must not turn Pole cleanup into Denied"
        );
        assert!(!meta.has_maps());
        meta.close_maps(None).unwrap();
        if fail_deallocate {
            assert_eq!(team::oust(&owner, child.id), Ok(()));
        }
        pend::release(&owner, token).unwrap();
        drop(view);
        drop(meta);
        drop(authorization);
        drop(child);
        messenger::slay(&owner);
        drop(owner);
        drop(parent);
        messenger::bury();
        scheduler::roster::prune_dead();
        assert!(held.upgrade().is_none());
        assert_eq!(messenger::probe_bookkeeping().1, husks);
        assert_eq!(messenger::site_count(), sites);
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
        assert_eq!(held_pages(), frames);
    }
}

pub fn pole_open_tracks_cpu_without_view_control() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let husks = messenger::probe_bookkeeping().1;
    for fail_open in [false, true] {
        let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
            .spawn()
            .unwrap();
        let owner = parent.task().hold().unwrap();
        let frame = Frame::allocate().unwrap();
        let held = Arc::downgrade(&frame);
        let input = support::map_frames(&parent.space, FIRST, vec![frame], rw(), rw()).unwrap();
        let (token, meta) = support::source_pole(&owner, &parent.space, &input, true);
        drop(input);
        let authorization = meta.authorization().unwrap();
        let source = Arc::downgrade(&authorization);
        let target = space();
        let asid = target.nest.asid();
        if fail_open {
            fail_shootdown(asid);
        }
        let opened = crate::core::work::mail::pole::open(&meta, token, &target, rw());
        let id = if fail_open {
            assert_eq!(opened, Err(env::PieFail::Busy));
            target.nest.active_ranges().unwrap()[0].0.identity()
        } else {
            let (address, size) = opened.unwrap();
            assert_eq!(size, PAGE_SIZE);
            target
                .nest
                .probe(VirtAddr::wrap(address))
                .unwrap()
                .identity()
        };
        let entry = target.views.registered(id).expect("Pole CPU binding");
        assert!(entry.control.is_none());
        assert_eq!(entry.view.is_none(), fail_open);
        assert!(target.views.locate(id).is_none());
        assert!(authorization.owns_binding(&entry.binding));
        assert_eq!(authorization.binding_count(), 2);
        let binding = entry.binding.clone();
        drop(entry);

        if !fail_open {
            fail_shootdown(asid);
        }
        let shut = crate::core::work::mail::pole::shut(&meta, token);
        assert_eq!(
            shut,
            if fail_open {
                Ok(())
            } else {
                Err(env::PieFail::Busy)
            }
        );
        assert_eq!(authorization.binding_count(), 1);
        pend::release(&owner, token).unwrap();
        drop(meta);
        drop(authorization);
        assert_eq!(source.upgrade().unwrap().binding_count(), 1);
        assert!(held.upgrade().is_some());
        assert!(target.views.registered(id).is_some());

        restore_shootdown(asid);
        target.nest.retry_invalidation().unwrap();
        target.reap_views();
        assert!(target.views.registered(id).is_none());
        assert!(source.upgrade().is_none());
        assert!(held.upgrade().is_none());
        target.retire().unwrap();
        drop(target);
        assert!(binding.space().upgrade().is_none());
        drop(binding);
        drop(source);
        messenger::slay(&owner);
        drop(owner);
        drop(parent);
        messenger::bury();
        scheduler::roster::prune_dead();
        assert_eq!(messenger::probe_bookkeeping().1, husks);
        assert_eq!(messenger::site_count(), sites);
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
        assert_eq!(held_pages(), frames);
    }
}
