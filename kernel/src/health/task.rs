#![cfg(debug_assertions)]

use crate::memory::manager::{MapError, addr::VirtAddr};
use crate::work::room::conductor;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;

pub fn preparation() {
    let space = SpaceBuilder::user().build().unwrap();
    space.with(|inner| inner.dynamic(0x4000_0000));
    let team = TeamBuilder::new(space).spawn().unwrap();
    let counts = conductor::counts();
    let tables = team.space.table_count();
    for _ in 0..4 {
        let prepared = team
            .task()
            .entry(VirtAddr::wrap(0x4000_0000))
            .prepare()
            .unwrap();
        assert_eq!(conductor::counts(), counts);
        assert!(team.tasks.lock().is_empty());
        assert!(team.held.lock().is_empty());
        drop(prepared);
        assert_eq!(team.space.table_count(), tables);
        assert_eq!(conductor::counts(), counts);
    }
    for stage in 1..=4 {
        crate::work::unit::task::fail_preparation_at(stage);
        assert!(matches!(team.task().prepare(), Err(MapError::OutOfMemory)));
        assert_eq!(conductor::counts(), counts);
        assert_eq!(team.space.table_count(), tables);
        assert!(team.tasks.lock().is_empty());
    }
    assert!(matches!(
        team.task().stack(usize::MAX).prepare(),
        Err(MapError::NoRegion)
    ));
    assert_eq!(conductor::counts(), counts);
}

pub fn construction() {
    use crate::memory::PAGE_SIZE;
    use crate::memory::manager::entry::PteFlags;
    use crate::work::mail::pole;
    use crate::work::room::scheduler;
    use crate::work::unit::space::{Backing, SegmentKind, Span};
    use crate::work::unit::{
        gate::{self, AnyPie},
        team::{self, Staging},
    };
    use alloc::{sync::Arc, vec::Vec};
    use core::sync::atomic::Ordering;
    use env::{Mark, Permission, UnitFail};
    scheduler::boot::init().expect("scheduler init");
    let parent_space = SpaceBuilder::supervisor().build().unwrap();
    parent_space.with(|inner| inner.dynamic(PAGE_SIZE));
    let parent = TeamBuilder::new(parent_space).spawn().unwrap();
    let caller = parent.task().hold().unwrap();
    let child = crate::work::unit::build(
        crate::work::unit::space::SpaceKind::User,
        crate::work::unit::weak::TaskWeak::empty(),
    )
    .unwrap();
    let code = Backing::allocate(PAGE_SIZE).unwrap();
    child.space.with_flush(|inner| {
        inner
            .allocate(
                crate::work::unit::space::SegmentKind::Normal,
                0x10000,
                PAGE_SIZE,
            )
            .unwrap();
        inner
            .backed(
                VirtAddr::wrap(0x10000),
                code,
                0,
                PAGE_SIZE,
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::U,
                PteFlags::R | PteFlags::X,
            )
            .unwrap();
    });
    let mut roots = Vec::new();
    let mut backings = Vec::new();
    for (index, access) in [PteFlags::R, PteFlags::R | PteFlags::W]
        .into_iter()
        .enumerate()
    {
        let meta = pole::meta(PAGE_SIZE, caller.ident.id).unwrap();
        let root: gate::Pie<gate::Pole> = gate::new_pie(
            meta.clone(),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
            None,
        );
        let token = root.token;
        caller.pies.lock().push(AnyPie::Pole(root));
        let open = parent
            .space
            .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
        pole::open(&meta, token, &parent.space, open).unwrap();
        assert!(meta.mapped());
        pole::shut(&meta, token).unwrap();
        assert!(!meta.mapped());
        let at = VirtAddr::wrap(0x20000 + index * PAGE_SIZE);
        child.space.with_flush(|inner| {
            inner
                .allocate(
                    crate::work::unit::space::SegmentKind::Normal,
                    at.as_usize(),
                    PAGE_SIZE,
                )
                .unwrap();
            inner
                .backed(
                    at,
                    meta.backing().clone(),
                    0,
                    PAGE_SIZE,
                    PteFlags::V | PteFlags::U | access,
                    access,
                )
                .unwrap();
            inner.bind(at, token);
            inner.private(at);
        });
        if !access.contains(PteFlags::W) {
            gate::narrow(
                caller
                    .pies
                    .lock()
                    .iter_mut()
                    .find(|pie| pie.token() == token)
                    .unwrap(),
                Permission::FETCH | Permission::VEST | Permission::ONLY,
            )
            .unwrap();
        }
        backings.push(Arc::downgrade(meta.backing()));
        meta.backing().reserve(child.id);
        child.staged.lock().push(Staging {
            token,
            meta,
            span: Span::new(SegmentKind::Normal, at, PAGE_SIZE, None),
        });
        roots.push(token);
    }
    let counts = conductor::counts();
    let tables = child.space.table_count();
    for (entry, stack, expected) in [
        (0, 0, UnitFail::BadEntry),
        (0x10001, 0, UnitFail::BadEntry),
        (0x10000, usize::MAX, UnitFail::Denied),
    ] {
        assert!(
            matches!(team::spawn(&child, Some(&caller), entry, Vec::new(), stack), Err(error) if error.code() == expected.code())
        );
        assert!(!child.ready.load(Ordering::Acquire));
        assert_eq!(child.default_entry(), 0);
        assert_eq!(conductor::counts(), counts);
        assert_eq!(child.space.table_count(), tables);
        for token in &roots {
            assert!(gate::locate(&caller, *token).is_some());
        }
    }
    scheduler::core::fail_next_reservation();
    assert!(matches!(
        team::spawn(&child, Some(&caller), 0x10000, Vec::new(), 0),
        Err(UnitFail::OoM)
    ));
    assert_eq!(conductor::counts(), counts);
    assert_eq!(child.space.table_count(), tables);
    assert!(!child.ready.load(Ordering::Acquire));
    assert_eq!(child.default_entry(), 0);
    assert_eq!(child.staged.lock().len(), 2);
    assert!(matches!(
        gate::release(&caller, roots[0]),
        Err(env::PieFail::Busy)
    ));
    let task = team::spawn(&child, Some(&caller), 0x10000, Vec::new(), 0).unwrap();
    assert!(child.ready.load(Ordering::Acquire));
    let frame_pa = task.ident.frame.pa.expect("task frame").as_usize();
    // SAFETY: the task has not run and its frame is exclusively owned by this test.
    let frame = unsafe { &*(frame_pa as *const crate::runtime::switcher::context::TrapContext) };
    assert_eq!(
        frame.gpr.x(crate::runtime::switcher::context::Gprs::SP),
        crate::memory::manager::mode::upper().as_usize()
    );
    assert_eq!(child.default_entry(), 0x10000);
    assert!(child.staged.lock().is_empty());
    assert!(
        scheduler::core::muster(task.ident.id)
            .unwrap()
            .upgrade()
            .is_some()
    );
    for token in roots {
        assert!(gate::locate(&caller, token).is_none());
    }
    for weak in &backings {
        assert!(weak.upgrade().is_some());
    }
    assert!(child.space.protect_user(0x20000, PAGE_SIZE, 6).is_err());
    assert!(child.space.validate_write(0x21000, PAGE_SIZE));
    assert!(child.release_held(&task));
    drop(task);
    drop(child);
    for weak in &backings {
        assert!(weak.upgrade().is_none());
    }
    assert_eq!(conductor::counts(), (counts.0 + 1, counts.1 + 1));
    assert!(parent.release_held(&caller));
    drop(caller);
}

pub fn cancellation() {
    use crate::memory::PAGE_SIZE;
    use crate::memory::manager::entry::PteFlags;
    use crate::work::mail::pole;
    use crate::work::unit::space::{SegmentKind, Span};
    use crate::work::unit::{
        gate::{self, AnyPie},
        team::Staging,
    };
    use alloc::sync::Arc;
    use env::{Mark, Permission};
    let parent_space = SpaceBuilder::supervisor().build().unwrap();
    parent_space.with(|inner| inner.dynamic(PAGE_SIZE));
    let parent = TeamBuilder::new(parent_space).spawn().unwrap();
    let caller = parent.task().hold().unwrap();
    let child = crate::work::unit::build(
        crate::work::unit::space::SpaceKind::User,
        crate::work::unit::weak::TaskWeak::empty(),
    )
    .unwrap();
    caller.adopt(child.clone()).unwrap();
    let meta = pole::meta(3 * PAGE_SIZE, caller.ident.id).unwrap();
    let root: gate::Pie<gate::Pole> = gate::new_pie(
        meta.clone(),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    );
    let token = root.token;
    caller.pies.lock().push(AnyPie::Pole(root));
    let at = VirtAddr::wrap(0x20000);
    child.space.with_flush(|inner| {
        inner
            .allocate(
                crate::work::unit::space::SegmentKind::Normal,
                at.as_usize(),
                3 * PAGE_SIZE,
            )
            .unwrap();
        inner
            .backed(
                at,
                meta.backing().clone(),
                0,
                3 * PAGE_SIZE,
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U,
                PteFlags::R | PteFlags::W,
            )
            .unwrap();
        inner.bind(at, token);
        inner.private(at);
    });
    meta.backing().reserve(child.id);
    child.staged.lock().push(Staging {
        token,
        meta: meta.clone(),
        span: Span::new(SegmentKind::Normal, at, 3 * PAGE_SIZE, None),
    });
    gate::reduce(
        &caller,
        token,
        Permission::FETCH | Permission::VEST | Permission::ONLY,
    )
    .unwrap();
    assert!(!child.space.validate_write(at.as_usize(), PAGE_SIZE));
    child
        .space
        .protect_user(at.as_usize() + PAGE_SIZE, PAGE_SIZE, 2)
        .unwrap();
    assert!(
        child
            .space
            .protect_user(at.as_usize() + PAGE_SIZE, PAGE_SIZE, 6)
            .is_err()
    );
    gate::reduce(&caller, token, Permission::FETCH | Permission::ONLY).unwrap();
    assert!(matches!(
        gate::release(&caller, token),
        Err(env::PieFail::Busy)
    ));
    let counts = conductor::counts();
    child.cancel_staging().unwrap();
    assert_eq!(conductor::counts(), counts);
    assert_eq!(meta.backing().reserved(), 0);
    assert!(child.space.translate(at).is_none());
    let remaining = gate::locate(&caller, token).unwrap().permission();
    assert_eq!(remaining, Permission::FETCH | Permission::ONLY);
    let (view, _) = pole::open(
        &meta,
        token,
        &parent.space,
        parent.space.pte_policy(PteFlags::V | PteFlags::R),
    )
    .unwrap();
    assert!(!parent.space.validate_write(view, PAGE_SIZE));
    pole::shut(&meta, token).unwrap();
    gate::release(&caller, token).unwrap();
    assert!(gate::locate(&caller, token).is_none());
    assert!(Arc::strong_count(meta.backing()) >= 1);
    drop(caller.oust(child.id));
    assert!(parent.release_held(&caller));
    drop(caller);
}
