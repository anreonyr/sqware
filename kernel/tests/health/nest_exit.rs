use ::core::sync::atomic::Ordering;
use alloc::sync::{Arc, Weak};
use alloc::vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::flat::allocator::statistics::{self, block_occupied, frame_occupied};
use crate::core::memory::nest::identity::{
    Asid, fail_next_shootdown, fail_shootdown, restore_shootdown,
};
use crate::core::memory::nest::table::TABLE_LIVE;
use crate::core::memory::nest::{Access, NestError, SpaceKind};
use crate::core::work::pend::{self, SourceAuthorization};
use crate::core::work::room::{conductor, messenger, scheduler};
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::task::{Task, TaskWeak};
use crate::core::work::unit::team::{self, Team, TeamBuilder};

use super::support;

const ADDRESS: VirtAddr = VirtAddr::wrap(0x6500_0000);

fn held_pages() -> usize {
    frame_occupied() - block_occupied()
}

fn task_with_view() -> (Arc<Team>, Arc<Task>, Weak<Frame>) {
    const HOOKS: &[fn(&Arc<Task>)] = &[pend::doom];
    messenger::hook(HOOKS);
    let team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let task = team.task().hold().unwrap();
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let rw = Access::READ | Access::WRITE;
    let source = support::frames_source(vec![frame]).unwrap();
    let authorization = SourceAuthorization::new(source.clone(), rw, task.ident.id).unwrap();
    let reservation = support::reserve(&team.space, ADDRESS, PAGE_SIZE).unwrap();
    let id = reservation.id();
    team.space
        .views
        .claim(
            id,
            &team.space,
            task.ident.id,
            env::ViewControl::Caller,
            authorization,
            None,
        )
        .unwrap();
    task.track_control(&team.space, id).unwrap();
    let view = team.space.nest.attach(reservation, source, rw, rw).unwrap();
    team.space.views.commit(id, view);
    (team, task, held)
}

pub fn retirement_releases_control_records() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    for fail_deallocate in [false, true] {
        let space = Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap();
        let asid = space.nest.asid();
        let frame = Frame::allocate().unwrap();
        let held = Arc::downgrade(&frame);
        let rw = Access::READ | Access::WRITE;
        let view = support::map_frames(&space, ADDRESS, vec![frame], rw, rw).unwrap();
        let id = view.identity();
        let authorization =
            SourceAuthorization::new(view.source().clone(), rw, env::TaskId::new(0)).unwrap();
        let source = Arc::downgrade(&authorization);
        space
            .views
            .claim(
                id,
                &space,
                env::TaskId::new(0),
                env::ViewControl::SpaceMembers,
                authorization,
                None,
            )
            .unwrap();
        space.views.commit(id, view);
        if fail_deallocate {
            space.nest.close().unwrap();
        }
        fail_shootdown(asid);
        assert_eq!(space.retire(), Err(NestError::Shootdown));
        assert!(!space.nest.retired());
        assert_eq!(space.nest.settled(id), fail_deallocate);
        assert_eq!(space.views.locate(id).is_some(), !fail_deallocate);
        assert_eq!(source.upgrade().is_some(), !fail_deallocate);
        assert_eq!(held.upgrade().is_some(), !fail_deallocate);
        assert!(space.nest.table_count() > 0);
        let other = SpaceBuilder::user().build().unwrap();
        assert_ne!(other.nest.asid(), asid);
        drop(other);

        restore_shootdown(asid);
        space.retire().unwrap();
        assert!(space.nest.retired());
        assert!(space.views.locate(id).is_none());
        assert!(source.upgrade().is_none());
        assert!(held.upgrade().is_none());
        space.retire().unwrap();
        drop(source);
        drop(held);
        drop(space);
        assert_eq!(messenger::site_count(), sites);
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
        assert_eq!(held_pages(), frames);
    }
    closed_space_releases_consumed_views();
    late_disposal_releases_consumed_view();
}

fn closed_space_releases_consumed_views() {
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let space = Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap();
    let asid = space.nest.asid();
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let rw = Access::READ | Access::WRITE;
    let view = support::map_frames(&space, ADDRESS, vec![frame], rw, rw).unwrap();
    let id = view.identity();
    fail_shootdown(asid);
    assert_eq!(space.dispose(view), Err(NestError::Shootdown));
    assert!(space.nest.contains_reservation(id));
    assert!(!space.nest.settled(id));
    assert!(held.upgrade().is_some());
    assert_eq!(space.nest.close(), Err(NestError::Shootdown));
    assert!(!space.nest.settled(id));
    assert!(held.upgrade().is_some());

    restore_shootdown(asid);
    space.nest.close().unwrap();
    assert!(!space.nest.retired());
    assert!(space.nest.settled(id));
    assert!(
        held.upgrade().is_none(),
        "reliable close must release consumed Views without waiting for the last Nest Arc"
    );
    space.nest.retry_invalidation().unwrap();
    let other = SpaceBuilder::user().build().unwrap();
    assert_ne!(other.nest.asid(), asid);
    drop(other);
    space.retire().unwrap();
    drop(space);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), frames);
}

fn late_disposal_releases_consumed_view() {
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let space = Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap();
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let rw = Access::READ | Access::WRITE;
    let view = support::map_frames(&space, ADDRESS, vec![frame], rw, rw).unwrap();
    let id = view.identity();
    space.nest.close().unwrap();
    assert!(space.nest.settled(id));
    assert!(held.upgrade().is_some());
    assert_eq!(space.dispose(view), Err(NestError::NoRegion));
    assert!(
        held.upgrade().is_none(),
        "a late consumed View must not be queued after reliable close"
    );
    space.nest.retry_invalidation().unwrap();
    space.retire().unwrap();
    drop(space);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), frames);
}

pub fn late_task_memory_settles_after_space_close() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let sites = messenger::site_count();
    let husks = messenger::probe_bookkeeping().1;
    let team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let task = team.task().hold().unwrap();
    let (id, view) = {
        let view = team.space.nest.probe(task.ident.frame_va).unwrap();
        (view.identity(), Arc::downgrade(&view))
    };
    let asid = team.space.nest.asid();
    messenger::slay(&task);
    assert!(task.reaped());
    fail_shootdown(asid);
    assert_eq!(team.space.nest.close(), Err(NestError::Shootdown));
    assert_eq!(task.ident.retire(), Err(NestError::Shootdown));
    assert!(!team.space.nest.settled(id));
    assert!(view.upgrade().is_some());

    restore_shootdown(asid);
    team.space.nest.close().unwrap();
    assert!(team.space.nest.contains_reservation(id));
    assert!(team.space.nest.settled(id));
    assert!(view.upgrade().is_some());
    assert_eq!(
        task.ident.retire(),
        Ok(()),
        "late TaskMemory retirement must not detach an already Closed relation"
    );
    assert!(view.upgrade().is_none());
    assert!(!team.space.nest.retired());
    task.ident.retire().unwrap();
    messenger::bury();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    team.retire().unwrap();
    drop(task);
    drop(team);
    messenger::bury();
    scheduler::roster::prune_dead();
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), frames);
}

pub fn task_exit_without_failure() {
    // 全局 ASID 位图及等待站点先初始化，再测单个任务的净回收。
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let kinds = statistics::kinds();
    let husks = messenger::probe_bookkeeping().1;
    let (team, task, held) = task_with_view();
    let asid = team.space.nest.asid();
    let nest = Arc::downgrade(&team.space.nest);
    messenger::slay(&task);
    assert!(task.reaped());
    assert!(team.space.nest.translate(ADDRESS).is_none());
    assert!(held.upgrade().is_none());
    drop(task);
    drop(team);
    messenger::bury();
    assert!(nest.upgrade().is_none());
    // 收尸时名册仍持有当前 Task 的弱引用；净回收前清掉已死项。
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(
        held_pages(),
        frames,
        "before: {kinds}; after: {}",
        statistics::kinds()
    );
    let replacement = SpaceBuilder::user().build().unwrap();
    assert_eq!(replacement.nest.asid(), asid);
    drop(replacement);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(held_pages(), frames);
}

pub fn task_exit_with_persistent_failure() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let kinds = statistics::kinds();
    let husks = messenger::probe_bookkeeping().1;
    let (team, task, held) = task_with_view();
    let source = {
        let view = team.space.nest.probe(ADDRESS).unwrap();
        let entry = team.space.views.locate(view.identity()).unwrap();
        Arc::downgrade(&entry.source)
    };
    let asid = team.space.nest.asid();
    let nest = Arc::downgrade(&team.space.nest);
    let retired_task = Arc::downgrade(&task);
    let installed = TABLE_LIVE.load(Ordering::Relaxed);
    let installed_frames = held_pages();
    fail_shootdown(asid);
    messenger::slay(&task);
    assert!(task.reaped());
    assert!(team.space.nest.translate(ADDRESS).is_none());
    for _ in 0..3 {
        assert_eq!(
            team.space.nest.retry_invalidation(),
            Err(NestError::Shootdown)
        );
        messenger::bury();
        assert_eq!(messenger::probe_bookkeeping().1, husks + 1);
        assert!(held.upgrade().is_some());
        assert!(source.upgrade().is_some());
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed);
        assert_eq!(held_pages(), installed_frames);
        assert_eq!(
            support::reserve(&team.space, ADDRESS, PAGE_SIZE).err(),
            Some(NestError::NoRegion)
        );
    }
    drop(task);
    drop(team);
    messenger::bury();
    assert!(retired_task.upgrade().is_some());
    assert!(nest.upgrade().is_some());
    assert!(held.upgrade().is_some());
    assert!(source.upgrade().is_some());
    assert_eq!(held_pages(), installed_frames);
    let replacement = SpaceBuilder::user().build().unwrap();
    assert_ne!(replacement.nest.asid(), asid);
    drop(replacement);
    restore_shootdown(asid);
    messenger::bury();
    assert!(retired_task.upgrade().is_none());
    assert!(nest.upgrade().is_none());
    assert!(held.upgrade().is_none());
    assert!(source.upgrade().is_none());
    drop(source);
    drop(retired_task);
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(
        held_pages(),
        frames,
        "before: {kinds}; after: {}",
        statistics::kinds()
    );
    let replacement = SpaceBuilder::user().build().unwrap();
    assert_eq!(replacement.nest.asid(), asid);
    drop(replacement);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(held_pages(), frames);
}

pub fn last_drop_with_persistent_failure() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let space = Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap();
    let asid = space.nest.asid();
    let nest = Arc::downgrade(&space.nest);
    let frame = Frame::allocate().unwrap();
    let held = Arc::downgrade(&frame);
    let rw = Access::READ | Access::WRITE;
    let view = support::map_frames(&space, ADDRESS, vec![frame], rw, rw).unwrap();
    let authorization =
        SourceAuthorization::new(view.source().clone(), rw, env::TaskId::new(0)).unwrap();
    let source = Arc::downgrade(&authorization);
    space
        .views
        .claim(
            view.identity(),
            &space,
            env::TaskId::new(0),
            env::ViewControl::SpaceMembers,
            authorization,
            None,
        )
        .unwrap();
    space.views.commit(view.identity(), view.clone());
    let installed = TABLE_LIVE.load(Ordering::Relaxed);
    let installed_frames = held_pages();
    fail_shootdown(asid);
    assert_eq!(space.nest.dispose(view), Err(NestError::Shootdown));
    drop(space);
    assert!(nest.upgrade().is_none());
    assert!(held.upgrade().is_some());
    assert!(source.upgrade().is_none());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed);
    assert_eq!(held_pages(), installed_frames);
    assert!(installed > before);
    restore_shootdown(asid);
    for _ in 0..3 {
        let replacement = SpaceBuilder::user().build().unwrap();
        assert_ne!(replacement.nest.asid(), asid);
        replacement.nest.retry_invalidation().unwrap();
        drop(replacement);
        messenger::bury();
        assert!(nest.upgrade().is_none());
        assert!(held.upgrade().is_some());
        assert!(source.upgrade().is_none());
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed);
        assert_eq!(held_pages(), installed_frames);
    }
    // 原始析构路径只验证禁止复用；保留资源不等于完成回收。
    crate::putln!(
        "nest_exit: raw last drop retains frames, not the source record; no reachable Nest"
    );
}

pub fn empty_team_oust_with_persistent_failure() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let husks = messenger::probe_bookkeeping().1;
    let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let task = parent.task().hold().unwrap();
    let parent_tables = TABLE_LIVE.load(Ordering::Relaxed);
    let child = TeamBuilder::build(
        SpaceKind::User,
        TaskWeak::stored(Arc::downgrade(&task), Origin::Sire),
    )
    .unwrap();
    let id = child.id;
    let asid = child.space.nest.asid();
    let nest = Arc::downgrade(&child.space.nest);
    let installed = TABLE_LIVE.load(Ordering::Relaxed);
    let retained = installed - parent_tables;
    assert!(retained > 0);
    assert!(child.all_reaped());
    assert_eq!(child.staged_len(), 0);
    drop(child);

    fail_shootdown(asid);
    let result = team::oust(&task, id);
    let lost_retirement = nest.upgrade().is_none();
    assert_eq!(result, Err(env::UnitFail::Busy));
    let retained_child = task.clan(id).expect("failed Oust retains its owner");
    assert!(retained_child.life.closed());
    assert!(retained_child.task().hold().is_err());
    for _ in 0..3 {
        assert_eq!(team::oust(&task, id), Err(env::UnitFail::Busy));
        assert!(Arc::ptr_eq(&task.clan(id).unwrap(), &retained_child));
    }
    drop(retained_child);
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed);
    let replacement = SpaceBuilder::user().build().unwrap();
    assert_ne!(replacement.nest.asid(), asid);
    drop(replacement);

    restore_shootdown(asid);
    if task.clan(id).is_some() {
        assert_eq!(team::oust(&task, id), Ok(()));
    }
    for _ in 0..3 {
        let replacement = SpaceBuilder::user().build().unwrap();
        replacement.nest.retry_invalidation().unwrap();
        drop(replacement);
        messenger::bury();
    }
    assert!(nest.upgrade().is_none());

    messenger::slay(&task);
    drop(task);
    drop(parent);
    messenger::bury();
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert!(
        !lost_retirement,
        "Oust returned {result:?} and lost the Team/Nest retry path after shootdown failure; retained tables: {}",
        TABLE_LIVE.load(Ordering::Relaxed) - before
    );
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

pub fn oust_retries_asid_deallocation() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let husks = messenger::probe_bookkeeping().1;
    let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let owner = parent.task().hold().unwrap();
    let child = TeamBuilder::build(
        SpaceKind::User,
        TaskWeak::stored(Arc::downgrade(&owner), Origin::Sire),
    )
    .unwrap();
    let id = child.id;
    let asid = child.space.nest.asid();
    child.space.nest.close().unwrap();
    assert!(!child.space.nest.retired());
    let closed_tables = TABLE_LIVE.load(Ordering::Relaxed);
    let closed_frames = held_pages();

    // close 已完成；下一次 shootdown 只可能来自 ASID 归还。
    fail_next_shootdown(asid);
    assert_eq!(team::oust(&owner, id), Err(env::UnitFail::Busy));
    assert!(Arc::ptr_eq(&owner.clan(id).unwrap(), &child));
    assert_eq!(child.id, id);
    assert!(child.life.closed());
    assert!(!child.space.nest.retired());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), closed_tables);
    assert_eq!(held_pages(), closed_frames);
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    let unavailable = SpaceBuilder::user().build().unwrap();
    assert_ne!(unavailable.nest.asid(), asid);
    drop(unavailable);

    assert_eq!(team::oust(&owner, id), Ok(()));
    assert!(owner.clan(id).is_none());
    assert!(child.space.nest.retired());
    let replacement = SpaceBuilder::user().build().unwrap();
    assert_eq!(replacement.nest.asid(), asid);
    for _ in 0..3 {
        child.retire().unwrap();
        assert_eq!(team::oust(&owner, id), Err(env::UnitFail::Denied));
    }
    drop(child);
    // 重试及旧 owner 析构都不能把 replacement 占住的号再次归还。
    let distinct = SpaceBuilder::user().build().unwrap();
    assert_ne!(distinct.nest.asid(), asid);
    drop(distinct);
    drop(replacement);
    let recycled = SpaceBuilder::user().build().unwrap();
    assert_eq!(recycled.nest.asid(), asid);
    drop(recycled);

    messenger::slay(&owner);
    drop(owner);
    drop(parent);
    messenger::bury();
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(held_pages(), frames);
}

pub fn retired_owner_preserves_reused_asid_watchers() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let husks = messenger::probe_bookkeeping().1;
    let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let owner = parent.task().hold().unwrap();
    let child = TeamBuilder::build(
        SpaceKind::User,
        TaskWeak::stored(Arc::downgrade(&owner), Origin::Sire),
    )
    .unwrap();
    let old_space = child.space.clone();
    let old_nest = Arc::downgrade(&old_space.nest);
    let asid = old_space.nest.asid();
    assert_eq!(team::oust(&owner, child.id), Ok(()));
    let replacement = SpaceBuilder::user().build().unwrap();
    assert_eq!(replacement.nest.asid(), asid);
    let life = replacement.life();
    let keys = [
        messenger::WakeKey::Space {
            space: asid,
            slot: 0x51,
        },
        messenger::WakeKey::View {
            space: asid,
            view: 0x52,
        },
    ];
    let toles = [owner.ident.id.get() * 2, owner.ident.id.get() * 2 + 1];
    for (key, tole) in keys.into_iter().zip(toles) {
        messenger::forward(key, life.clone(), tole, life.clone()).unwrap();
    }
    assert_eq!(messenger::site_count(), sites + keys.len());

    for _ in 0..3 {
        child.retire().unwrap();
        old_space.retire().unwrap();
        assert_eq!(messenger::site_count(), sites + keys.len());
    }
    drop(child);
    assert!(old_nest.upgrade().is_some());
    assert_eq!(messenger::site_count(), sites + keys.len());
    drop(old_space);
    assert!(old_nest.upgrade().is_none());
    assert_eq!(messenger::site_count(), sites + keys.len());

    // 实际 signal 经 watch 转发到 Tole，wait 消费其提示；不只核对站点数量。
    for (key, tole) in keys.into_iter().zip(toles) {
        assert_eq!(messenger::signal(key), 0);
        assert_eq!(messenger::site_count(), sites + keys.len() + 1);
        assert!(matches!(
            messenger::wait::<env::RoomFail>(
                messenger::WakeKey::Tole { id: tole },
                life.clone(),
                ::core::time::Duration::MAX,
            ),
            Ok(messenger::Handoff::Resume(()))
        ));
        assert_eq!(messenger::site_count(), sites + keys.len());
    }
    for (key, tole) in keys.into_iter().zip(toles) {
        messenger::unforward(key, tole);
    }
    assert_eq!(messenger::site_count(), sites);
    drop(life);
    drop(replacement);
    let recycled = SpaceBuilder::user().build().unwrap();
    assert_eq!(recycled.nest.asid(), asid);
    drop(recycled);
    messenger::slay(&owner);
    drop(owner);
    drop(parent);
    messenger::bury();
    drop(old_nest);
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(held_pages(), frames);
}

pub fn parent_exit_retains_empty_child() {
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let husks = messenger::probe_bookkeeping().1;
    let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let task = parent.task().hold().unwrap();
    let child = TeamBuilder::build(
        SpaceKind::User,
        TaskWeak::stored(Arc::downgrade(&task), Origin::Sire),
    )
    .unwrap();
    let asid = child.space.nest.asid();
    let nest = Arc::downgrade(&child.space.nest);
    let owner = Arc::downgrade(&task);
    drop(child);
    fail_shootdown(asid);
    messenger::slay(&task);
    drop(task);
    drop(parent);
    for _ in 0..3 {
        messenger::bury();
        assert!(owner.upgrade().is_some());
        assert!(nest.upgrade().is_some());
        assert_eq!(messenger::probe_bookkeeping().1, husks + 1);
    }
    restore_shootdown(asid);
    messenger::bury();
    assert!(owner.upgrade().is_none());
    assert!(nest.upgrade().is_none());
    drop(owner);
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

pub fn all_reaped_preserves_respawn() {
    let team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let first = team.task().hold().unwrap();
    messenger::slay(&first);
    drop(first);
    messenger::bury();
    assert!(team.all_reaped());
    assert!(!team.life.closed());
    let second = team.task().hold().expect("reaped Live team may respawn");
    assert!(!team.all_reaped());
    assert_eq!(team.retire(), Err(NestError::NoRegion));
    assert!(!team.life.closed());
    messenger::slay(&second);
    drop(second);
    drop(team);
    messenger::bury();
    scheduler::roster::prune_dead();
}

pub fn oust_retires_pruned_task_memory() {
    let sites = messenger::site_count();
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let husks = messenger::probe_bookkeeping().1;
    let parent = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let owner = parent.task().hold().unwrap();
    let child = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .sire(TaskWeak::stored(Arc::downgrade(&owner), Origin::Sire))
        .spawn()
        .unwrap();
    let task = child.task().hold().unwrap();
    let asid = child.space.nest.asid();
    assert_eq!(team::oust(&owner, child.id), Err(env::UnitFail::Busy));
    assert!(!child.life.closed());
    messenger::slay(&task);
    {
        let _commit = crate::core::work::commit();
        child.life.clear(owner.ident.id, None);
        child.life.prune();
    }
    assert!(child.life.tasks.lock().is_empty());
    fail_shootdown(asid);
    assert_eq!(team::oust(&owner, child.id), Err(env::UnitFail::Busy));
    assert!(owner.clan(child.id).is_some());
    assert!(!child.space.nest.retired());
    assert_eq!(messenger::probe_bookkeeping().1, husks + 1);
    restore_shootdown(asid);
    assert_eq!(team::oust(&owner, child.id), Ok(()));
    assert!(child.space.nest.retired());
    drop(task);
    drop(child);
    messenger::bury();
    messenger::slay(&owner);
    drop(owner);
    drop(parent);
    messenger::bury();
    scheduler::roster::prune_dead();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

pub fn task_exit_resumes_retirement_on_schedule() {
    scheduler::init().unwrap();
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let husks = messenger::probe_bookkeeping().1;
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = held_pages();
    let (team, task, held) = task_with_view();
    let asid = team.space.nest.asid();
    let nest = Arc::downgrade(&team.space.nest);
    let retired_task = Arc::downgrade(&task);
    let task_pa = task.ident.frame_pa.as_usize();

    let observer_team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let observer = observer_team.task().hold().unwrap();
    let observer_pa = observer.ident.frame_pa.as_usize();
    // quit 必须有可返回的任务；最后一枚只用于夹具收尾。
    let fallback = observer_team.task().hold().unwrap();
    let fallback_pa = fallback.ident.frame_pa.as_usize();

    Task::release(&task).unwrap();
    assert_eq!(conductor::run::run(), task_pa);
    assert!(task.on_hart().is_some());
    Task::release(&observer).unwrap();
    fail_shootdown(asid);
    drop(task);
    drop(team);
    assert_eq!(messenger::quit(), observer_pa);
    assert_eq!(messenger::probe_bookkeeping().1, husks + 1);
    assert!(retired_task.upgrade().is_some());
    assert!(nest.upgrade().is_some());
    assert!(held.upgrade().is_some());
    let failed_tables = TABLE_LIVE.load(Ordering::Relaxed);
    let failed_frames = held_pages();
    for _ in 0..3 {
        assert_eq!(conductor::run::run(), observer_pa);
        assert_eq!(conductor::current().starve(), observer_pa);
        assert!(held.upgrade().is_some());
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), failed_tables);
        assert_eq!(held_pages(), failed_frames);
    }

    restore_shootdown(asid);
    for _ in 0..16 {
        assert_eq!(conductor::run::run(), observer_pa);
        assert_eq!(conductor::current().starve(), observer_pa);
    }
    let recovered_on_schedule =
        retired_task.upgrade().is_none() && nest.upgrade().is_none() && held.upgrade().is_none();

    Task::release(&fallback).unwrap();
    drop(observer);
    assert_eq!(messenger::quit(), fallback_pa);
    assert!(retired_task.upgrade().is_none());
    assert!(nest.upgrade().is_none());
    assert!(held.upgrade().is_none());
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    drop(retired_task);

    let (last, next) = conductor::current().swap();
    assert!(next.is_none());
    assert!(Arc::ptr_eq(&last, &fallback));
    last.state.lock().doom(None, env::ExitCause::Slay, 0);
    drop(last);
    drop(fallback);
    drop(observer_team);
    scheduler::roster::prune_dead();
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
    assert_eq!(held_pages(), frames);
    assert!(
        recovered_on_schedule,
        "retirement resumed only after another Task exited, not after normal scheduling"
    );
}
