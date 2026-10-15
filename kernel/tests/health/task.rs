#![cfg(debug_assertions)]

use alloc::sync::Arc;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::nest::{Access, NestError, RegionId, View, ViewRange};
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::core::work::unit::tally;
use crate::core::work::unit::team::TeamBuilder;

use super::support;

/// 一段页对齐窗口在某个关系上的身份选择。
fn select(view: &Arc<View>, start: VirtAddr, size: usize) -> ViewRange {
    let offset = (start.as_usize() - view.initial_start().as_usize()) / PAGE_SIZE;
    view.range(
        PageOffset::new(offset),
        PageCount::new(size / PAGE_SIZE).expect("task: page count"),
    )
    .expect("task: selection")
}

fn own_pages(space: &Space, start: VirtAddr, size: usize, access: Access) -> Arc<View> {
    use crate::core::memory::flat::Frame;
    use alloc::vec::Vec;
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(size / PAGE_SIZE)
        .expect("owned frame list");
    for _ in 0..size / PAGE_SIZE {
        frames.push(crate::tag!(Pole, Frame::allocate().expect("owned frame")));
    }
    // 帧先备齐，再登记占用：来源准备失败不留无人持有的地址。
    let reservation = support::reserve(space, start, size).expect("reserve owned pages");
    support::attach_frames(space, reservation, frames, access, access).expect("map owned pages")
}

fn stage_only(
    child: &alloc::sync::Arc<crate::core::work::unit::team::Team>,
    meta: &alloc::sync::Arc<crate::core::work::mail::pole::PoleMeta>,
    token: env::PieToken,
    access: Access,
    start: VirtAddr,
) -> alloc::sync::Arc<crate::core::memory::nest::View> {
    meta.reserve_records(1).expect("reserve Pole record");
    child.reserve_staging(1).expect("reserve staged record");
    let source = meta.window(0, meta.size()).expect("Pole source window");
    let reservation =
        support::reserve(&child.space, start, meta.size()).expect("reserve staged range");
    let id = reservation.id();
    let authorization = meta.authorization().expect("source authorization");
    child
        .space
        .views
        .claim(
            id,
            &child.space,
            meta.owner(),
            env::ViewControl::Caller,
            authorization,
            Some(token),
        )
        .expect("staged identity");
    let view = child
        .space
        .nest
        .attach(reservation, source, access, access)
        .expect("attach staged map");
    child.space.views.commit(id, view.clone());
    assert!(
        meta.record(token, &child.space, view.clone(), Some(child.id), false)
            .is_ok()
    );
    child.push_staging(crate::core::work::unit::team::Staging {
        token,
        meta: meta.clone(),
        view: view.clone(),
    });
    view
}

pub fn preparation() {
    let space = SpaceBuilder::user().build().unwrap();
    space
        .nest
        .attach_region(
            crate::core::memory::nest::RegionId::Normal,
            0x4000_0000,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let counts = tally::counts();
    let tables = team.space.nest.table_count();
    for _ in 0..4 {
        let prepared = team
            .task()
            .entry(VirtAddr::wrap(0x4000_0000))
            .prepare()
            .unwrap();
        assert_eq!(tally::counts(), counts);
        assert!(team.tasks.members.lock().is_empty());
        assert!(team.tasks.held.lock().is_empty());
        drop(prepared);
        assert_eq!(team.space.nest.table_count(), tables);
        assert_eq!(tally::counts(), counts);
    }
    for stage in 1..=4 {
        crate::core::work::unit::task::fail_preparation_at(stage);
        assert!(matches!(team.task().prepare(), Err(NestError::OutOfMemory)));
        assert_eq!(tally::counts(), counts);
        assert_eq!(team.space.nest.table_count(), tables);
        assert!(team.tasks.members.lock().is_empty());
    }
    let maps = team.space.nest.view_count();
    for stage in 1..=2 {
        crate::core::work::unit::task::fail_invalidation_at(stage);
        assert!(matches!(team.task().prepare(), Err(NestError::Shootdown)));
        assert_eq!(tally::counts(), counts);
        assert!(team.tasks.members.lock().is_empty());
        // 已发布的那一段由巢的清理责任收尾：重试前后都不留残留。
        team.space
            .nest
            .retry_invalidation()
            .expect("prepare: recover failed installation");
        assert_eq!(team.space.nest.view_count(), maps);
        assert_eq!(team.space.nest.table_count(), tables);
        let prepared = team.task().prepare().unwrap();
        drop(prepared);
        assert_eq!(team.space.nest.view_count(), maps);
        assert_eq!(team.space.nest.table_count(), tables);
    }
    let expected = support::reserve_low(
        &team.space,
        crate::core::memory::nest::RegionId::Kernel,
        PAGE_SIZE,
    )
    .unwrap();
    let first_frame = expected.start();
    team.space.nest.release_reservation(expected).unwrap();
    crate::core::work::unit::task::fail_preparation_at(3);
    crate::core::work::unit::task::fail_rollback_fence();
    assert!(matches!(team.task().prepare(), Err(NestError::OutOfMemory)));
    assert_eq!(tally::counts(), counts);
    let reserved = support::reserve_low(
        &team.space,
        crate::core::memory::nest::RegionId::Kernel,
        PAGE_SIZE,
    )
    .unwrap();
    assert_ne!(reserved.start(), first_frame);
    team.space.nest.release_reservation(reserved).unwrap();
    team.space.nest.retry_invalidation().unwrap();
    let reused = support::reserve_low(
        &team.space,
        crate::core::memory::nest::RegionId::Kernel,
        PAGE_SIZE,
    )
    .unwrap();
    assert_eq!(reused.start(), first_frame);
    team.space.nest.release_reservation(reused).unwrap();
    assert_eq!(team.space.nest.table_count(), tables);
    assert_eq!(team.space.nest.view_count(), maps);
    assert!(matches!(
        team.task().stack(usize::MAX).prepare(),
        Err(NestError::NoRegion)
    ));
    assert_eq!(tally::counts(), counts);
}

pub fn construction() {
    use crate::core::memory::PAGE_SIZE;
    use crate::core::memory::nest::table::PteFlags;
    use crate::core::work::mail::pole;
    use crate::core::work::room::scheduler;
    use crate::core::work::{pend, unit::team};
    use alloc::sync::{Arc, Weak};
    use alloc::vec::Vec;
    use env::{Permission, UnitFail};
    scheduler::init().expect("scheduler init");
    let parent_space = SpaceBuilder::supervisor().build().unwrap();
    parent_space
        .nest
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let parent = TeamBuilder::new(parent_space).spawn().unwrap();
    let caller = parent.task().hold().unwrap();
    let child = crate::core::work::unit::team::TeamBuilder::build(
        crate::core::work::unit::space::SpaceKind::User,
        crate::core::work::unit::task::TaskWeak::empty(),
    )
    .unwrap();
    caller.adopt(child.clone()).unwrap();
    child
        .space
        .nest
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let code = own_pages(
        &child.space,
        VirtAddr::wrap(0x10000),
        PAGE_SIZE,
        Access::READ | Access::EXECUTE,
    );
    assert_eq!(code.initial_range().0, VirtAddr::wrap(0x10000));
    let (_, code_flags) = child
        .space
        .nest
        .translate(VirtAddr::wrap(0x10000))
        .expect("owned entry mapping");
    assert!(code_flags.contains(PteFlags::R | PteFlags::X));
    assert!(!code_flags.contains(PteFlags::W));
    let mut roots = Vec::new();
    let mut frame_weak: Vec<Vec<Weak<crate::core::memory::flat::Frame>>> = Vec::new();
    let mut stage_maps = Vec::new();
    for (index, access) in [Access::READ, Access::READ | Access::WRITE]
        .into_iter()
        .enumerate()
    {
        let input = own_pages(
            &parent.space,
            VirtAddr::wrap(0x30000 + index * PAGE_SIZE),
            PAGE_SIZE,
            Access::READ | Access::WRITE,
        );
        let frames = input.frames_in(input.initial_start(), PAGE_SIZE).unwrap();
        frame_weak.push(
            frames
                .iter()
                .map(|(_, frame)| Arc::downgrade(frame))
                .collect(),
        );
        drop(frames);
        let (token, meta) = support::source_pole(&caller, &parent.space, &input, false);
        drop(input);
        let (open_va, _) =
            pole::open(&meta, token, &parent.space, Access::READ | Access::WRITE).unwrap();
        assert!(
            parent
                .space
                .nest
                .translate(VirtAddr::wrap(open_va))
                .is_some()
        );
        assert!(meta.mapped());
        pole::shut(&meta, token).unwrap();
        assert!(!meta.mapped());
        assert!(
            parent
                .space
                .nest
                .translate(VirtAddr::wrap(open_va))
                .is_none()
        );
        let at = VirtAddr::wrap(0x20000 + index * PAGE_SIZE);
        let staged = stage_only(&child, &meta, token, access, at);
        stage_maps.push(staged);
        if !access.contains(Access::WRITE) {
            pend::reduce(
                &caller,
                token,
                Permission::FETCH | Permission::VEST | Permission::ONLY,
            )
            .unwrap();
        }
        roots.push(token);
    }
    let counts = tally::counts();
    let tables = child.space.nest.table_count();
    for (entry, stack, expected) in [
        (0, 0, UnitFail::BadEntry),
        (0x10001, 0, UnitFail::BadEntry),
        (0x10000, usize::MAX, UnitFail::Denied),
    ] {
        assert!(
            matches!(team::spawn(&child, Some(&caller), entry, Vec::new(), stack), Err(error) if error.code() == expected.code())
        );
        assert!(!child.ready());
        assert_eq!(child.default_entry(), 0);
        assert_eq!(tally::counts(), counts);
        assert_eq!(child.space.nest.table_count(), tables);
        for token in &roots {
            assert!(pend::locate(&caller, *token).is_some());
        }
    }
    scheduler::roster::fail_next_reservation();
    assert!(matches!(
        team::spawn(&child, Some(&caller), 0x10000, Vec::new(), 0),
        Err(UnitFail::OoM)
    ));
    assert_eq!(tally::counts(), counts);
    assert_eq!(child.space.nest.table_count(), tables);
    assert!(!child.ready());
    assert_eq!(child.default_entry(), 0);
    assert_eq!(child.staged_len(), 2);
    assert!(matches!(
        pend::release(&caller, roots[0]),
        Err(env::PieFail::Busy)
    ));
    let task = team::spawn(&child, Some(&caller), 0x10000, Vec::new(), 0).unwrap();
    assert!(child.ready());
    let frame_pa = task.ident.frame_pa.as_usize();
    let frame = unsafe { &*(frame_pa as *const crate::machine::switcher::context::TrapContext) };
    assert_eq!(
        frame.gpr.x(crate::machine::switcher::context::Gprs::SP),
        crate::machine::mode::upper().as_usize()
    );
    assert_eq!(child.default_entry(), 0x10000);
    assert!(child.staged_len() == 0);
    assert!(
        scheduler::roster::muster(task.ident.id)
            .unwrap()
            .upgrade()
            .is_some()
    );
    for token in roots {
        assert!(pend::locate(&caller, token).is_none());
        let call = env::MemoryCall::Acquire {
            source: env::AcquireSource::Source {
                token,
                offset: 0,
                count: 1,
            },
            retention: env::PieToken::NONE,
        };
        let frame = unsafe {
            &mut *(caller.ident.frame_pa.as_usize()
                as *mut crate::machine::switcher::context::TrapContext)
        };
        for (at, value) in call.pack().into_iter().enumerate() {
            frame
                .gpr
                .set_x(crate::machine::switcher::context::Gprs::A0 + at, value);
        }
        frame
            .gpr
            .set_x(crate::machine::switcher::context::Gprs::A7, call.slot());
        assert!(crate::entry::call::dispatch(frame, caller.ident.clone()).is_some());
        assert_eq!(
            frame.gpr.x(crate::machine::switcher::context::Gprs::A0) as isize,
            env::MemoryFail::Denied.code(),
            "the consumed sender token cannot acquire another source hold"
        );
    }
    for view in &stage_maps {
        let entry = child.space.views.locate(view.identity()).unwrap();
        assert_eq!(entry.controller, task.ident.id);
        assert!(entry.branch.is_none());
        assert!(entry.source.own_range().is_none());
        assert!(!entry.source.branch_check(None, Access::READ));
        assert_eq!(
            entry
                .source
                .enter_publish(view.identity(), None, Access::READ),
            Err(env::PieFail::Denied),
            "ONLY delivery does not restore source publication or creation"
        );
        assert_eq!(entry.source.claim(), Err(env::PieFail::Denied));
    }
    for va in [VirtAddr::wrap(0x20000), VirtAddr::wrap(0x21000)] {
        let (_, flags) = child
            .space
            .nest
            .translate(va)
            .expect("published staged map");
        assert!(flags.contains(PteFlags::R));
        assert_eq!(flags.contains(PteFlags::W), va == VirtAddr::wrap(0x21000));
    }
    assert!(
        !child
            .space
            .validate_access(VirtAddr::wrap(0x20000), PAGE_SIZE, Access::WRITE)
    );
    assert!(
        child
            .space
            .validate_access(VirtAddr::wrap(0x21000), PAGE_SIZE, Access::WRITE)
    );
    drop(stage_maps);
    for weak_frames in &frame_weak {
        assert!(weak_frames.iter().all(|weak| weak.upgrade().is_some()));
    }
    assert!(
        child
            .space
            .validate_access(VirtAddr::wrap(0x21000), PAGE_SIZE, Access::WRITE)
    );
    assert!(child.release_held(&task));
    drop(task);
    drop(child);
    assert_eq!(tally::counts(), (counts.0 + 1, counts.1 + 1));
    assert!(parent.release_held(&caller));
    drop(caller);
    for weak_frames in &frame_weak {
        assert!(weak_frames.iter().all(|weak| weak.upgrade().is_none()));
    }
}

pub fn cancellation() {
    use crate::core::memory::PAGE_SIZE;
    use crate::core::memory::nest::table::PteFlags;
    use crate::core::work::mail::pole;
    use crate::core::work::pend;
    use alloc::sync::{Arc, Weak};
    use alloc::vec::Vec;
    use env::Permission;
    let parent_space = SpaceBuilder::supervisor().build().unwrap();
    parent_space
        .nest
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let parent = TeamBuilder::new(parent_space).spawn().unwrap();
    let caller = parent.task().hold().unwrap();
    let child = crate::core::work::unit::team::TeamBuilder::build(
        crate::core::work::unit::space::SpaceKind::User,
        crate::core::work::unit::task::TaskWeak::empty(),
    )
    .unwrap();
    caller.adopt(child.clone()).unwrap();
    child
        .space
        .nest
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let input = own_pages(
        &parent.space,
        VirtAddr::wrap(0x30000),
        3 * PAGE_SIZE,
        Access::READ | Access::WRITE,
    );
    let frames = input
        .frames_in(input.initial_start(), 3 * PAGE_SIZE)
        .unwrap();
    let weak_frames: Vec<Weak<crate::core::memory::flat::Frame>> = frames
        .iter()
        .map(|(_, frame)| Arc::downgrade(frame))
        .collect();
    drop(frames);
    let (token, meta) = support::source_pole(&caller, &parent.space, &input, false);
    drop(input);
    pole::shut(&meta, token).unwrap();
    let at = VirtAddr::wrap(0x20000);
    let staged_map = stage_only(&child, &meta, token, Access::READ | Access::WRITE, at);
    pend::reduce(
        &caller,
        token,
        Permission::FETCH | Permission::VEST | Permission::ONLY,
    )
    .unwrap();
    assert!(!child.space.validate_access(at, PAGE_SIZE, Access::WRITE));
    child
        .space
        .nest
        .set_access(select(&staged_map, at + PAGE_SIZE, PAGE_SIZE), Access::READ)
        .unwrap();
    assert!(
        child
            .space
            .nest
            .set_access(
                select(&staged_map, at + PAGE_SIZE, PAGE_SIZE),
                Access::READ | Access::WRITE,
            )
            .is_err()
    );
    pend::reduce(&caller, token, Permission::FETCH | Permission::ONLY).unwrap();
    assert!(matches!(
        pend::release(&caller, token),
        Err(env::PieFail::Busy)
    ));
    let counts = tally::counts();
    child.cancel_staging().unwrap();
    assert_eq!(tally::counts(), counts);
    assert_eq!(meta.reserved(), 0);
    assert!(child.space.nest.translate(at).is_none());
    let remaining = pend::locate(&caller, token).unwrap().permission();
    assert_eq!(remaining, Permission::FETCH | Permission::ONLY);
    let (view, _) = pole::open(&meta, token, &parent.space, Access::READ).unwrap();
    let (_, view_flags) = parent
        .space
        .nest
        .translate(VirtAddr::wrap(view))
        .expect("read-only Pole view");
    assert!(!view_flags.contains(PteFlags::W));
    child.cancel_staging().unwrap();
    pole::shut(&meta, token).unwrap();
    pend::release(&caller, token).unwrap();
    assert!(pend::locate(&caller, token).is_none());
    drop(caller.oust(child.id));
    drop(staged_map);
    drop(meta);
    assert!(weak_frames.iter().all(|weak| weak.upgrade().is_none()));
    assert!(parent.release_held(&caller));
    drop(caller);
}

pub fn branch_control() {
    use crate::core::memory::nest::RegionId;
    use crate::core::work::unit::copies::Origin;
    use crate::core::work::unit::task::{Task, TaskExitCause, TaskState, TaskWeak};
    use alloc::sync::Arc;
    fn team(parent: Option<&Arc<Task>>) -> Arc<crate::core::work::unit::team::Team> {
        let space = SpaceBuilder::user().build().unwrap();
        space
            .nest
            .advance_region(RegionId::Normal, 0x4000_0000)
            .unwrap();
        let builder = TeamBuilder::new(space);
        match parent {
            Some(parent) => builder.sire(TaskWeak::stored(Arc::downgrade(parent), Origin::Sire)),
            None => builder,
        }
        .spawn()
        .unwrap()
    }
    let root = team(None);
    let main = root.task().hold().unwrap();
    let peer = root.task().hold().unwrap();
    let child = team(Some(&main));
    let member = child.task().hold().unwrap();
    root.debark().unwrap();
    assert!(root.paused() && child.paused());
    let later = team(Some(&member));
    let grandchild = later.task().hold().unwrap();
    assert!(later.paused());
    assert!(main.heir(later.id).is_none());
    assert!(Arc::ptr_eq(&main.clan(later.id).unwrap(), &later));
    assert!(member.clan(child.id).is_none());
    assert!(crate::core::work::unit::join::observe(&main, grandchild.ident.id.into()).is_ok());
    child.debark().unwrap();
    root.embark().unwrap();
    assert!(!root.paused() && child.paused() && later.paused());
    child.embark().unwrap();
    assert!(!child.paused() && !later.paused());
    let record = later.life.member(grandchild.ident.id).unwrap();
    assert!(Arc::ptr_eq(&peer.clan(later.id).unwrap(), &later));
    let target = crate::core::work::unit::join::observe(&peer, grandchild.ident.id.into()).unwrap();
    assert_eq!(
        crate::core::work::unit::join::JoinWait::new(target, peer.ident.id, true, env::Wait::POLL)
            .poll(),
        Err(env::UnitFail::Denied)
    );
    let target = crate::core::work::unit::join::observe(&peer, later.id.into()).unwrap();
    assert_eq!(
        crate::core::work::unit::join::JoinWait::new(target, peer.ident.id, true, env::Wait::POLL)
            .poll(),
        Err(env::UnitFail::Denied)
    );
    assert!(record.pending(main.ident.id) && record.pending(member.ident.id));
    assert!(record.exit().is_none());
    use crate::core::work::unit::join::{JoinTarget, JoinWait};
    let observe = JoinWait::new(
        JoinTarget::Task(record.clone()),
        main.ident.id,
        false,
        env::Wait::POLL,
    );
    let receive = JoinWait::new(
        JoinTarget::Task(record.clone()),
        member.ident.id,
        true,
        env::Wait::POLL,
    );
    assert_eq!(observe.poll(), Ok(env::JoinReply::Pending));
    let space = Arc::downgrade(&later.space);
    later.release_held(&grandchild);
    *grandchild.state.lock() = TaskState::reaped(TaskExitCause::Reap, 17);
    tally::exit();
    assert_eq!(record.exit().unwrap().reason, 17);
    assert_eq!(observe.poll(), observe.poll());
    assert!(receive.poll().unwrap().is_reaped());
    assert_eq!(receive.poll(), Err(env::UnitFail::Denied));
    assert!(record.pending(main.ident.id));
    member.oust(later.id);
    let closed = JoinWait::new(
        JoinTarget::Team(later.life.clone()),
        member.ident.id,
        true,
        env::Wait::POLL,
    );
    assert_eq!(closed.poll(), Err(env::UnitFail::Denied));
    drop(closed);
    drop(grandchild);
    drop(later);
    assert!(
        space.upgrade().is_none(),
        "pending exits retained execution resources"
    );
    assert_eq!(
        child
            .life
            .next(main.ident.id)
            .unwrap()
            .exit()
            .unwrap()
            .reason,
        17
    );
    let parent = JoinWait::new(
        JoinTarget::Team(child.life.clone()),
        main.ident.id,
        true,
        env::Wait::POLL,
    );
    assert!(parent.poll().unwrap().is_reaped());
    assert_eq!(parent.poll(), Ok(env::JoinReply::Pending));
    assert!(
        observe.poll().unwrap().is_reaped(),
        "an enrolled observer lost the shared exit"
    );
    child.life.prune();
    drop(parent);
    drop(receive);
    drop(observe);
    drop(record);
    child.release_held(&member);
    *member.state.lock() = TaskState::reaped(TaskExitCause::Reap, 0);
    tally::exit();
    child.life.clear(main.ident.id, None);
    child.life.prune();
    main.oust(child.id);
    drop(member);
    drop(child);
    root.release_held(&peer);
    *peer.state.lock() = TaskState::reaped(TaskExitCause::Reap, 0);
    tally::exit();
    drop(peer);
    root.release_held(&main);
    *main.state.lock() = TaskState::reaped(TaskExitCause::Reap, 0);
    tally::exit();
    root.life.prune();
    drop(main);
    drop(root);
}
