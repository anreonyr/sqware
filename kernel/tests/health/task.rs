#![cfg(debug_assertions)]

use crate::core::memory::manager::{MapError, addr::VirtAddr};
use crate::core::work::room::conductor;
use crate::core::work::unit::space::{Mapping, RegionId, Space, SpaceBuilder};
use crate::core::work::unit::team::TeamBuilder;

fn own_pages(
    space: &Space,
    start: VirtAddr,
    size: usize,
    access: crate::core::memory::manager::entry::PteFlags,
) -> Mapping {
    use crate::core::memory::manager::{Frame, entry::PteFlags};
    use alloc::vec::Vec;
    let reservation = space
        .reserve(RegionId::Normal, start, size)
        .expect("reserve owned pages");
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(size / crate::core::memory::PAGE_SIZE)
        .expect("owned frame list");
    for _ in 0..size / crate::core::memory::PAGE_SIZE {
        frames.push(crate::tag!(Pole, Frame::allocate().expect("owned frame")));
    }
    let current = space.pte_policy(access | PteFlags::V | PteFlags::A | PteFlags::D);
    space
        .map_frames(reservation, frames, access, current)
        .expect("map owned pages")
}

fn stage_only(
    child: &alloc::sync::Arc<crate::core::work::unit::team::Team>,
    meta: &alloc::sync::Arc<crate::core::work::mail::pole::PoleMeta>,
    token: env::PieToken,
    access: crate::core::memory::manager::entry::PteFlags,
    start: VirtAddr,
) -> alloc::sync::Arc<crate::core::memory::manager::Map> {
    use crate::core::memory::manager::entry::PteFlags;
    meta.reserve_record().expect("reserve Pole record");
    child.reserve_staging(1).expect("reserve staged record");
    let source = meta.canonical_map().expect("canonical Pole map");
    let reservation = child
        .space
        .reserve(RegionId::Normal, start, meta.size())
        .expect("reserve staged range");
    let mapping = child
        .space
        .derive(
            reservation,
            &source,
            0,
            access,
            child
                .space
                .pte_policy(access | PteFlags::V | PteFlags::A | PteFlags::D),
        )
        .expect("derive staged map");
    let map = mapping.map.clone();
    assert!(
        meta.record(token, &child.space, mapping, Some(child.id), false)
            .is_ok()
    );
    child.push_staging(crate::core::work::unit::team::Staging {
        token,
        meta: meta.clone(),
        map: map.clone(),
    });
    map
}

pub fn preparation() {
    let space = SpaceBuilder::user().build().unwrap();
    space
        .attach_region(
            crate::core::work::unit::space::RegionId::Normal,
            0x4000_0000,
            crate::core::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let counts = conductor::counts();
    let tables = team.space.tree.table_count();
    for _ in 0..4 {
        let prepared = team
            .task()
            .entry(VirtAddr::wrap(0x4000_0000))
            .prepare()
            .unwrap();
        assert_eq!(conductor::counts(), counts);
        assert!(team.tasks.members.lock().is_empty());
        assert!(team.tasks.held.lock().is_empty());
        drop(prepared);
        assert_eq!(team.space.tree.table_count(), tables);
        assert_eq!(conductor::counts(), counts);
    }
    for stage in 1..=4 {
        crate::core::work::unit::task::fail_preparation_at(stage);
        assert!(matches!(team.task().prepare(), Err(MapError::OutOfMemory)));
        assert_eq!(conductor::counts(), counts);
        assert_eq!(team.space.tree.table_count(), tables);
        assert!(team.tasks.members.lock().is_empty());
    }
    let maps = team.space.tree.map_count();
    for stage in 1..=2 {
        crate::core::work::unit::task::fail_invalidation_at(stage);
        assert!(matches!(team.task().prepare(), Err(MapError::Shootdown)));
        assert_eq!(conductor::counts(), counts);
        assert!(team.tasks.members.lock().is_empty());
        assert!(team.space.tree.map_count() > maps);
        team.space
            .retry_invalidation()
            .expect("prepare: recover failed installation");
        assert_eq!(team.space.tree.map_count(), maps);
        assert_eq!(team.space.tree.table_count(), tables);
        let prepared = team.task().prepare().unwrap();
        drop(prepared);
        assert_eq!(team.space.tree.map_count(), maps);
        assert_eq!(team.space.tree.table_count(), tables);
    }
    let expected = team
        .space
        .allocate_low(
            crate::core::work::unit::space::RegionId::Kernel,
            crate::core::memory::PAGE_SIZE,
        )
        .unwrap();
    let first_frame = expected.start();
    team.space.release_reservation(expected).unwrap();
    crate::core::work::unit::task::fail_preparation_at(3);
    crate::core::work::unit::task::fail_rollback_fence();
    assert!(matches!(team.task().prepare(), Err(MapError::OutOfMemory)));
    assert_eq!(conductor::counts(), counts);
    let reserved = team
        .space
        .allocate_low(
            crate::core::work::unit::space::RegionId::Kernel,
            crate::core::memory::PAGE_SIZE,
        )
        .unwrap();
    assert_ne!(reserved.start(), first_frame);
    team.space.release_reservation(reserved).unwrap();
    team.space.retry_invalidation().unwrap();
    let reused = team
        .space
        .allocate_low(
            crate::core::work::unit::space::RegionId::Kernel,
            crate::core::memory::PAGE_SIZE,
        )
        .unwrap();
    assert_eq!(reused.start(), first_frame);
    team.space.release_reservation(reused).unwrap();
    assert_eq!(team.space.tree.table_count(), tables);
    assert_eq!(team.space.tree.map_count(), maps);
    assert!(matches!(
        team.task().stack(usize::MAX).prepare(),
        Err(MapError::NoRegion)
    ));
    assert_eq!(conductor::counts(), counts);
}

pub fn construction() {
    use crate::core::memory::PAGE_SIZE;
    use crate::core::memory::manager::entry::PteFlags;
    use crate::core::work::mail::{Mail, pole};
    use crate::core::work::room::scheduler;
    use crate::core::work::{pend, unit::team};
    use alloc::sync::{Arc, Weak};
    use alloc::vec::Vec;
    use env::{Mark, Permission, UnitFail};
    scheduler::boot::init().expect("scheduler init");
    let parent_space = SpaceBuilder::supervisor().build().unwrap();
    parent_space
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::core::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let parent = TeamBuilder::new(parent_space).spawn().unwrap();
    let caller = parent.task().hold().unwrap();
    let child = crate::core::work::unit::build(
        crate::core::work::unit::space::SpaceKind::User,
        crate::core::work::unit::task::TaskWeak::empty(),
    )
    .unwrap();
    caller.adopt(child.clone()).unwrap();
    child
        .space
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::core::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let code = own_pages(
        &child.space,
        VirtAddr::wrap(0x10000),
        PAGE_SIZE,
        PteFlags::R | PteFlags::X,
    );
    assert_eq!(code.map.initial_range().0, VirtAddr::wrap(0x10000));
    let (_, code_flags) = child
        .space
        .tree
        .translate(VirtAddr::wrap(0x10000))
        .expect("owned entry mapping");
    assert!(code_flags.contains(PteFlags::R | PteFlags::X));
    assert!(!code_flags.contains(PteFlags::W));
    let mut roots = Vec::new();
    let mut frame_weak: Vec<Vec<Weak<crate::core::memory::manager::Frame>>> = Vec::new();
    let mut stage_maps = Vec::new();
    for (index, access) in [PteFlags::R, PteFlags::R | PteFlags::W]
        .into_iter()
        .enumerate()
    {
        let input = own_pages(
            &parent.space,
            VirtAddr::wrap(0x30000 + index * PAGE_SIZE),
            PAGE_SIZE,
            PteFlags::R | PteFlags::W,
        );
        let frames = input
            .map
            .frames_in(input.reservation.start(), PAGE_SIZE)
            .unwrap();
        frame_weak.push(
            frames
                .iter()
                .map(|(_, frame)| Arc::downgrade(frame))
                .collect(),
        );
        drop(frames);
        let meta = pole::PoleMeta::from_mapping(&parent.space, input, caller.ident.id).unwrap();
        let root = pend::Pie::try_new(
            Mail::Pole(meta.clone()),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
            None,
        )
        .expect("pie allocation");
        let token = root.token();
        pend::insert(&caller, root).unwrap();
        meta.commit_source();
        let open = parent
            .space
            .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
        let (open_va, _) = pole::open(&meta, token, &parent.space, open).unwrap();
        assert!(
            parent
                .space
                .tree
                .translate(VirtAddr::wrap(open_va))
                .is_some()
        );
        assert!(meta.mapped());
        pole::shut(&meta, token).unwrap();
        assert!(!meta.mapped());
        assert!(
            parent
                .space
                .tree
                .translate(VirtAddr::wrap(open_va))
                .is_none()
        );
        let at = VirtAddr::wrap(0x20000 + index * PAGE_SIZE);
        let staged = stage_only(&child, &meta, token, access, at);
        stage_maps.push(staged);
        if !access.contains(PteFlags::W) {
            pend::reduce(
                &caller,
                token,
                Permission::FETCH | Permission::VEST | Permission::ONLY,
            )
            .unwrap();
        }
        roots.push(token);
    }
    let counts = conductor::counts();
    let tables = child.space.tree.table_count();
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
        assert_eq!(conductor::counts(), counts);
        assert_eq!(child.space.tree.table_count(), tables);
        for token in &roots {
            assert!(pend::locate(&caller, *token).is_some());
        }
    }
    scheduler::core::fail_next_reservation();
    assert!(matches!(
        team::spawn(&child, Some(&caller), 0x10000, Vec::new(), 0),
        Err(UnitFail::OoM)
    ));
    assert_eq!(conductor::counts(), counts);
    assert_eq!(child.space.tree.table_count(), tables);
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
        crate::core::memory::manager::mode::upper().as_usize()
    );
    assert_eq!(child.default_entry(), 0x10000);
    assert!(child.staged_len() == 0);
    assert!(
        scheduler::core::muster(task.ident.id)
            .unwrap()
            .upgrade()
            .is_some()
    );
    for token in roots {
        assert!(pend::locate(&caller, token).is_none());
    }
    for va in [VirtAddr::wrap(0x20000), VirtAddr::wrap(0x21000)] {
        let (_, flags) = child
            .space
            .tree
            .translate(va)
            .expect("published staged map");
        assert!(flags.contains(PteFlags::R));
        assert_eq!(flags.contains(PteFlags::W), va == VirtAddr::wrap(0x21000));
    }
    assert!(!child.space.validate_write(0x20000, PAGE_SIZE));
    assert!(child.space.validate_write(0x21000, PAGE_SIZE));
    drop(stage_maps);
    for weak_frames in &frame_weak {
        assert!(weak_frames.iter().all(|weak| weak.upgrade().is_some()));
    }
    assert!(child.space.validate_write(0x21000, PAGE_SIZE));
    assert!(child.release_held(&task));
    drop(task);
    drop(child);
    assert_eq!(conductor::counts(), (counts.0 + 1, counts.1 + 1));
    assert!(parent.release_held(&caller));
    drop(caller);
    for weak_frames in &frame_weak {
        assert!(weak_frames.iter().all(|weak| weak.upgrade().is_none()));
    }
}

pub fn cancellation() {
    use crate::core::memory::PAGE_SIZE;
    use crate::core::memory::manager::entry::PteFlags;
    use crate::core::work::mail::{Mail, pole};
    use crate::core::work::pend;
    use alloc::sync::{Arc, Weak};
    use alloc::vec::Vec;
    use env::{Mark, Permission};
    let parent_space = SpaceBuilder::supervisor().build().unwrap();
    parent_space
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::core::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let parent = TeamBuilder::new(parent_space).spawn().unwrap();
    let caller = parent.task().hold().unwrap();
    let child = crate::core::work::unit::build(
        crate::core::work::unit::space::SpaceKind::User,
        crate::core::work::unit::task::TaskWeak::empty(),
    )
    .unwrap();
    caller.adopt(child.clone()).unwrap();
    child
        .space
        .attach_region(
            RegionId::Normal,
            PAGE_SIZE,
            crate::core::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let input = own_pages(
        &parent.space,
        VirtAddr::wrap(0x30000),
        3 * PAGE_SIZE,
        PteFlags::R | PteFlags::W,
    );
    let frames = input
        .map
        .frames_in(input.reservation.start(), 3 * PAGE_SIZE)
        .unwrap();
    let weak_frames: Vec<Weak<crate::core::memory::manager::Frame>> = frames
        .iter()
        .map(|(_, frame)| Arc::downgrade(frame))
        .collect();
    drop(frames);
    let meta = pole::PoleMeta::from_mapping(&parent.space, input, caller.ident.id).unwrap();
    let root = pend::Pie::try_new(
        Mail::Pole(meta.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )
    .expect("pie allocation");
    let token = root.token();
    pend::insert(&caller, root).unwrap();
    meta.commit_source();
    pole::shut(&meta, token).unwrap();
    let at = VirtAddr::wrap(0x20000);
    let staged_map = stage_only(&child, &meta, token, PteFlags::R | PteFlags::W, at);
    pend::reduce(
        &caller,
        token,
        Permission::FETCH | Permission::VEST | Permission::ONLY,
    )
    .unwrap();
    assert!(!child.space.validate_write(at.as_usize(), PAGE_SIZE));
    child
        .space
        .tree
        .protect(
            &staged_map,
            at + PAGE_SIZE,
            PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::U,
        )
        .unwrap();
    assert!(
        child
            .space
            .tree
            .protect(
                &staged_map,
                at + PAGE_SIZE,
                PAGE_SIZE,
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D | PteFlags::U,
            )
            .is_err()
    );
    pend::reduce(&caller, token, Permission::FETCH | Permission::ONLY).unwrap();
    assert!(matches!(
        pend::release(&caller, token),
        Err(env::PieFail::Busy)
    ));
    let counts = conductor::counts();
    child.cancel_staging().unwrap();
    assert_eq!(conductor::counts(), counts);
    assert_eq!(meta.reserved(), 0);
    assert!(child.space.tree.translate(at).is_none());
    let remaining = pend::locate(&caller, token).unwrap().permission();
    assert_eq!(remaining, Permission::FETCH | Permission::ONLY);
    let (view, _) = pole::open(
        &meta,
        token,
        &parent.space,
        parent.space.pte_policy(PteFlags::V | PteFlags::R),
    )
    .unwrap();
    let (_, view_flags) = parent
        .space
        .tree
        .translate(VirtAddr::wrap(view))
        .expect("read-only Pole view");
    assert!(!view_flags.contains(PteFlags::W));
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
    use crate::core::work::unit::copies::Origin;
    use crate::core::work::unit::space::RegionId;
    use crate::core::work::unit::task::{Task, TaskExitCause, TaskState, TaskWeak};
    use alloc::sync::Arc;
    fn team(parent: Option<&Arc<Task>>) -> Arc<crate::core::work::unit::team::Team> {
        let space = SpaceBuilder::user().build().unwrap();
        space.advance_region(RegionId::Normal, 0x4000_0000).unwrap();
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
        crate::core::work::unit::join::JoinWait::new(target, peer.ident.id, true, env::Wait::POLL).poll(),
        Err(env::UnitFail::Denied)
    );
    let target = crate::core::work::unit::join::observe(&peer, later.id.into()).unwrap();
    assert_eq!(
        crate::core::work::unit::join::JoinWait::new(target, peer.ident.id, true, env::Wait::POLL).poll(),
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
    conductor::exit();
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
    conductor::exit();
    child.life.clear(main.ident.id, None);
    child.life.prune();
    main.oust(child.id);
    drop(member);
    drop(child);
    root.release_held(&peer);
    *peer.state.lock() = TaskState::reaped(TaskExitCause::Reap, 0);
    conductor::exit();
    drop(peer);
    root.release_held(&main);
    *main.state.lock() = TaskState::reaped(TaskExitCause::Reap, 0);
    conductor::exit();
    root.life.prune();
    drop(main);
    drop(root);
}
