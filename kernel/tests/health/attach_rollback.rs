//! `Attach` 回滚：撤图责任先交给 Nest，占用票结清前登记、来源与控制都在。
//!
//! 两个失败形态都要成立：发布尝试失败（清理链还在 Nest 手里）与已落地后的
//! 撤图失败（同步失败）。恢复后只沿既有 `retry_invalidation` / `reap_views` 收尾，
//! 不另造重试队列；成功可达的回滚则当场清掉，不等下一轮。

#[path = "attach_rollback_fixture.rs"]
mod fixture;

use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageOffset, VirtAddr as KVirt};
use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
use crate::core::memory::nest::identity::{
    Asid, fail_next_shootdown, fail_shootdown, restore_shootdown,
};
use crate::core::memory::nest::table::TABLE_LIVE;
use crate::core::memory::nest::{Access, NestError, SpaceKind};
use crate::core::work::room::messenger;
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::task::TaskWeak;
use crate::core::work::unit::team::{self, TeamBuilder};
use crate::entry::call::memory::rollback;

use super::support;
use fixture::Fixture;

/// 被测关系与预留的地址。
const TARGET: usize = 0x6500_0000;

fn rw() -> Access {
    Access::READ | Access::WRITE
}

/// 已尝试发布后失败（清理链留在 Nest）：原始失败码原样返回，登记与控制等到结清。
fn attempt_failure_keeps_control_until_settled() {
    let fixture = Fixture::new();
    let asid = fixture.team.space.nest.asid();
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    let reference = fixture
        .task
        .pend
        .references
        .insert(
            authorization.clone(),
            PageOffset::ZERO,
            range.page_count(),
            Access::all(),
            None,
            None,
        )
        .unwrap();
    drop(range);

    // 发布已经写进页表与索引，最后的失效确认失败：Nest 留下清理链。
    fail_shootdown(asid);
    assert_eq!(
        fixture.attach(reference, TARGET, env::TeamId::new(0)),
        env::MemoryFail::Busy.code(),
        "the original failure code is kept"
    );
    let id = {
        let ranges = fixture.team.space.nest.active_ranges().unwrap();
        let failed = ranges
            .iter()
            .find(|(_, range)| range.start.as_usize() == TARGET)
            .expect("the failed publish is still the one relation");
        failed.0.identity()
    };
    assert!(fixture.team.space.nest.contains_reservation(id));
    assert!(fixture.team.space.views.pending(id).is_some());
    assert!(fixture.task.holds_control(id));
    assert_eq!(authorization.binding_count(), 1);

    // 引用撤走后只剩登记里的控制记录在强持来源；帧由 Nest 的清理链持有。
    assert!(fixture.task.pend.references.remove(reference).is_some());
    drop(authorization);
    assert!(
        source.upgrade().is_some(),
        "the pending control keeps its source"
    );
    assert!(
        held.upgrade().is_some(),
        "the cleanup chain keeps the frame"
    );

    restore_shootdown(asid);
    fixture.team.space.nest.retry_invalidation().unwrap();
    fixture.team.space.reap_views();
    assert!(!fixture.team.space.nest.contains_reservation(id));
    assert!(fixture.team.space.views.pending(id).is_none());
    assert!(!fixture.task.holds_control(id));
    assert!(
        fixture
            .team
            .space
            .nest
            .active_ranges()
            .unwrap()
            .iter()
            .all(|(_, range)| range.start.as_usize() != TARGET)
    );
    assert!(
        source.upgrade().is_none(),
        "settled: the source record is gone"
    );
    assert!(held.upgrade().is_none(), "settled: the frame is gone");
    fixture.finish();
}

/// 已落地关系的撤图同步失败：登记与控制保留到 `close_controls` 沿既有路径结清。
fn dispose_failure_keeps_control_for_task_close() {
    let fixture = Fixture::new();
    let asid = fixture.team.space.nest.asid();
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    let (id, reservation) = fixture.claim(&authorization);
    let view = fixture.publish(reservation, range);

    fail_shootdown(asid);
    rollback(&fixture.team.space, id, None, Some(view));
    assert!(fixture.team.space.nest.contains_reservation(id));
    assert!(fixture.team.space.views.pending(id).is_some());
    assert!(fixture.task.holds_control(id));
    assert!(source.upgrade().is_some());
    assert!(held.upgrade().is_some());
    assert_eq!(authorization.binding_count(), 1);

    assert_eq!(fixture.task.close_controls(), Err(NestError::Shootdown));
    assert!(fixture.task.holds_control(id));
    assert!(fixture.team.space.views.pending(id).is_some());
    assert_eq!(authorization.binding_count(), 1);

    // 任务退场不是把它当成不存在：恢复后 `close_controls` 自己把清理链走完。
    restore_shootdown(asid);
    fixture.task.close_controls().unwrap();
    assert!(!fixture.team.space.nest.contains_reservation(id));
    assert!(fixture.team.space.views.pending(id).is_none());
    assert!(!fixture.task.holds_control(id));
    assert_eq!(authorization.binding_count(), 0);
    drop(authorization);
    assert!(source.upgrade().is_none());
    assert!(held.upgrade().is_none());
    fixture.finish();
}

fn foreign_control_failure_keeps_task_in_husks() {
    let fixture = Fixture::new();
    let target = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    let reservation = support::reserve(&target.space, KVirt::wrap(TARGET), PAGE_SIZE).unwrap();
    let id = reservation.id();
    target
        .space
        .views
        .claim(
            id,
            &target.space,
            fixture.task.ident.id,
            env::ViewControl::Caller,
            authorization.clone(),
            None,
        )
        .unwrap();
    fixture.task.track_control(&target.space, id).unwrap();
    let view = target
        .space
        .nest
        .attach(reservation, range, rw(), rw())
        .unwrap();
    drop(authorization);
    let husks = messenger::probe_bookkeeping().1;
    let asid = target.space.nest.asid();
    fail_shootdown(asid);
    rollback(&target.space, id, None, Some(view));
    messenger::slay(&fixture.task);
    assert!(fixture.task.reaped());
    messenger::bury();
    assert_eq!(messenger::probe_bookkeeping().1, husks + 1);
    assert!(fixture.task.holds_control(id));
    assert!(source.upgrade().is_some());
    assert!(held.upgrade().is_some());

    restore_shootdown(asid);
    messenger::bury();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert!(!fixture.task.holds_control(id));
    assert!(target.space.views.pending(id).is_none());
    assert!(source.upgrade().is_none());
    assert!(held.upgrade().is_none());
    drop(source);
    drop(held);
    drop(target);
    fixture.finish();
}

fn closed_child_retires_before_owner_controls() {
    let fixture = Fixture::new();
    let child = TeamBuilder::build(
        SpaceKind::User,
        TaskWeak::stored(Arc::downgrade(&fixture.task), Origin::Sire),
    )
    .unwrap();
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    let reservation = support::reserve(&child.space, KVirt::wrap(TARGET), PAGE_SIZE).unwrap();
    let id = reservation.id();
    child
        .space
        .views
        .claim(
            id,
            &child.space,
            fixture.task.ident.id,
            env::ViewControl::Caller,
            authorization,
            None,
        )
        .unwrap();
    fixture.task.track_control(&child.space, id).unwrap();
    let view = child
        .space
        .nest
        .attach(reservation, range, rw(), rw())
        .unwrap();
    child.space.views.commit(id, view);
    let husks = messenger::probe_bookkeeping().1;
    let asid = child.space.nest.asid();
    child.space.nest.close().unwrap();
    fail_next_shootdown(asid);
    assert_eq!(
        team::oust(&fixture.task, child.id),
        Err(env::UnitFail::Busy)
    );
    assert!(!child.space.nest.retired());
    assert!(child.space.nest.contains_reservation(id));
    assert!(child.space.nest.settled(id));
    assert!(!fixture.task.holds_control(id));
    assert!(source.upgrade().is_none());
    assert!(held.upgrade().is_none());

    // Oust 的一次性失败已消耗；收尸必须先推进子团队，不能等关闭空间的票自行消失。
    messenger::slay(&fixture.task);
    messenger::bury();
    assert!(
        child.space.nest.retired(),
        "the owner's controls must not block the child's ASID retirement"
    );
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert!(!fixture.task.holds_control(id));
    assert!(child.space.views.registered(id).is_none());
    assert!(source.upgrade().is_none());
    drop(child);
    fixture.finish();
    assert!(held.upgrade().is_none());
}

fn closed_foreign_child_releases_caller_without_retiring_space(owner_reaps: bool) {
    let fixture = Fixture::new();
    let owner = fixture.team.task().hold().unwrap();
    let child = TeamBuilder::build(
        SpaceKind::User,
        TaskWeak::stored(Arc::downgrade(&owner), Origin::Sire),
    )
    .unwrap();
    assert!(fixture.task.heir(child.id).is_none());
    assert!(Arc::ptr_eq(&fixture.task.clan(child.id).unwrap(), &child));
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    let reference = fixture
        .task
        .pend
        .references
        .insert(
            authorization,
            PageOffset::ZERO,
            range.page_count(),
            Access::all(),
            None,
            None,
        )
        .unwrap();
    drop(range);
    let id = usize::try_from(fixture.attach(reference, TARGET, child.id))
        .expect("Caller Attach into another owner's constructing child");
    assert!(fixture.task.pend.references.remove(reference).is_some());
    assert!(child.space.views.locate(id).is_some());
    assert!(fixture.task.holds_control(id));

    let husks = messenger::probe_bookkeeping().1;
    let asid = child.space.nest.asid();
    child.space.nest.close().unwrap();
    fail_next_shootdown(asid);
    if owner_reaps {
        assert_eq!(team::oust(&owner, child.id), Err(env::UnitFail::Busy));
    } else {
        assert_eq!(child.space.nest.retire(), Err(NestError::Shootdown));
    }
    assert!(!child.space.nest.retired());
    assert!(child.space.nest.contains_reservation(id));
    assert!(child.space.nest.settled(id));
    assert_eq!(child.space.views.locate(id).is_some(), !owner_reaps);
    assert_eq!(fixture.task.holds_control(id), !owner_reaps);
    assert_eq!(source.upgrade().is_some(), !owner_reaps);
    assert_eq!(held.upgrade().is_some(), !owner_reaps);
    messenger::slay(&fixture.task);
    messenger::bury();
    assert_eq!(
        messenger::probe_bookkeeping().1,
        husks,
        "a closed foreign Space must not strand its Caller in HUSKS"
    );
    assert!(!fixture.task.holds_control(id));
    assert!(child.space.views.registered(id).is_none());
    assert!(source.upgrade().is_none());
    assert!(held.upgrade().is_none());
    assert!(
        !child.space.nest.retired(),
        "Caller cleanup does not acquire authority to return the foreign ASID"
    );

    assert_eq!(team::oust(&owner, child.id), Ok(()));
    messenger::slay(&owner);
    drop(owner);
    drop(child);
    fixture.finish();
}

fn reaping_controls_respects_cpu_settlement() {
    let fixture = Fixture::new();
    let other = fixture.team.task().hold().unwrap();
    for closed in [false, true] {
        let target = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
            .spawn()
            .unwrap();
        let mut registrations = alloc::vec::Vec::new();
        for (index, controller) in [&fixture.task, &other, &fixture.task]
            .into_iter()
            .enumerate()
        {
            let address = KVirt::wrap(TARGET + index * PAGE_SIZE);
            let (authorization, range, held) = fixture.source();
            let source = Arc::downgrade(&authorization);
            let reservation = support::reserve(&target.space, address, PAGE_SIZE).unwrap();
            let id = reservation.id();
            target
                .space
                .views
                .claim(
                    id,
                    &target.space,
                    controller.ident.id,
                    env::ViewControl::Caller,
                    authorization,
                    None,
                )
                .unwrap();
            controller.track_control(&target.space, id).unwrap();
            let view = target
                .space
                .nest
                .attach(reservation, range, rw(), rw())
                .unwrap();
            target.space.views.commit(id, view);
            registrations.push((id, source, held));
        }
        if closed {
            target.space.nest.close().unwrap();
        }
        fixture.task.close_controls().unwrap();
        for (index, (id, source, held)) in registrations.iter().enumerate() {
            let remains = index == 1 && !closed;
            assert_eq!(target.space.views.registered(*id).is_some(), remains);
            assert_eq!(source.upgrade().is_some(), remains);
            assert_eq!(held.upgrade().is_some(), remains);
            assert!(!fixture.task.holds_control(*id));
        }
        assert_eq!(other.holds_control(registrations[1].0), !closed);
        assert!(!target.space.nest.retired());
        if !closed {
            assert!(
                target
                    .space
                    .nest
                    .translate(KVirt::wrap(TARGET + PAGE_SIZE))
                    .is_some(),
                "another controller's live mapping must survive"
            );
        }
        other.close_controls().unwrap();
        for (id, source, held) in registrations {
            assert!(target.space.views.registered(id).is_none());
            assert!(source.upgrade().is_none());
            assert!(held.upgrade().is_none());
        }
        drop(target);
    }
    messenger::slay(&other);
    drop(other);
    fixture.finish();
}

/// 撤图当场成功：不等下一轮，登记与控制立即撤掉。
fn successful_disposal_clears_immediately() {
    let fixture = Fixture::new();
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    let (id, reservation) = fixture.claim(&authorization);
    let view = fixture.publish(reservation, range);

    rollback(&fixture.team.space, id, None, Some(view));
    assert!(!fixture.team.space.nest.contains_reservation(id));
    assert!(fixture.team.space.views.pending(id).is_none());
    assert!(!fixture.task.holds_control(id));
    drop(authorization);
    assert!(source.upgrade().is_none());
    assert!(held.upgrade().is_none());
    fixture.finish();
}

/// 预发布失败：先交还预留，再撤预留槽；不能把它当已发布的 CPU 待清理。
fn pre_publish_failure_returns_the_span() {
    let fixture = Fixture::new();
    let (authorization, range, held) = fixture.source();
    let source = Arc::downgrade(&authorization);
    drop(range);
    let reservation =
        support::reserve(&fixture.team.space, KVirt::wrap(TARGET), PAGE_SIZE).unwrap();
    let id = reservation.id();
    fixture
        .team
        .space
        .views
        .claim(
            id,
            &fixture.team.space,
            fixture.task.ident.id,
            env::ViewControl::Caller,
            authorization.clone(),
            None,
        )
        .unwrap();
    fixture.task.track_control(&fixture.team.space, id).unwrap();

    rollback(&fixture.team.space, id, Some(reservation), None);
    assert!(!fixture.team.space.nest.contains_reservation(id));
    assert!(fixture.team.space.views.pending(id).is_none());
    assert!(!fixture.task.holds_control(id));
    // 预留先交还：同址能立刻重新登记，不留占用票。
    let reused = support::reserve(&fixture.team.space, KVirt::wrap(TARGET), PAGE_SIZE).unwrap();
    fixture.team.space.nest.release_reservation(reused).unwrap();
    drop(authorization);
    assert!(source.upgrade().is_none());
    assert!(held.upgrade().is_none());
    fixture.finish();
}

/// `Attach` 回滚的净账：两个失败形态都保留到结清，成功形态与预发布当场清。
pub fn preserves_control_until_settled() {
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let pages = frame_occupied() - block_occupied();
    let sites = messenger::site_count();
    attempt_failure_keeps_control_until_settled();
    dispose_failure_keeps_control_for_task_close();
    foreign_control_failure_keeps_task_in_husks();
    closed_child_retires_before_owner_controls();
    closed_foreign_child_releases_caller_without_retiring_space(false);
    closed_foreign_child_releases_caller_without_retiring_space(true);
    reaping_controls_respects_cpu_settlement();
    successful_disposal_clears_immediately();
    pre_publish_failure_returns_the_span();
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(frame_occupied() - block_occupied(), pages);
}
