use alloc::sync::Arc;

use env::{Mark, MemoryCall, Permission, PieFail, PieToken, TeamId};

use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::nest::Access;
use crate::core::memory::nest::identity::{fail_shootdown, restore_shootdown};
use crate::entry::call::pole::create_pole;

use super::baseline::Baseline;
use super::fixture::Fixture;
use super::{flat_reference, pend, token_of};

const MAPPED: VirtAddr = VirtAddr::wrap(0x6800_0000);

fn branch(fixture: &Fixture, parent: PieToken, offset: usize, count: usize) -> PieToken {
    token_of(
        pend::accord_window(
            &fixture.task,
            parent,
            &Arc::downgrade(&fixture.task),
            Permission::FETCH | Permission::VEST,
            Mark::NONE,
            Some((PageOffset::new(offset), PageCount::new(count).unwrap())),
        )
        .unwrap(),
    )
}

fn derive(fixture: &Fixture, parent: PieToken) -> PieToken {
    let reference = fixture.acquire(parent, 1, 1, PieToken::NONE);
    assert!(reference > 0);
    let token = create_pole(&fixture.task, flat_reference(reference), true).unwrap();
    assert_eq!(
        fixture
            .call(MemoryCall::Release {
                reference: flat_reference(reference),
            })
            .0,
        0
    );
    token
}

fn map(fixture: &Fixture, parent: PieToken) -> usize {
    let reference = fixture.acquire(parent, 0, 1, PieToken::NONE);
    assert!(reference > 0);
    let (id, address) = fixture.attach_to(reference, 0, 1, MAPPED, TeamId::new(0), Access::READ);
    assert!(id > 0);
    assert_eq!(address, MAPPED.as_usize());
    assert_eq!(
        fixture
            .call(MemoryCall::Release {
                reference: flat_reference(reference),
            })
            .0,
        0
    );
    id as usize
}

pub fn forget_preserves_derived_indices() {
    let baseline = Baseline::new();
    let fixture = Fixture::shared(4);
    let middle = branch(&fixture, fixture.token, 0, 3);
    let leaf = branch(&fixture, middle, 2, 1);
    let id = map(&fixture, middle);
    let control = fixture
        .team
        .space
        .views
        .locate(id)
        .unwrap()
        .control
        .unwrap();
    let child = derive(&fixture, middle);
    let (child_meta, child_source) = fixture.child(child);
    let (_, geometry) = child_source.lineage().unwrap();
    let child_permission = pend::locate(&fixture.task, child).unwrap().permission();
    let leaf_before = pend::locate(&fixture.task, leaf).unwrap();
    let leaf_permission = leaf_before.permission();
    let leaf_grant = leaf_before.grant().clone();
    drop(leaf_before);

    let asid = fixture.team.space.nest.asid();
    fail_shootdown(asid);
    pend::forget(&fixture.task, middle).unwrap();
    assert!(pend::locate(&fixture.task, middle).is_none());
    assert_eq!(
        fixture.authorization.derivation_token(&geometry),
        Some(fixture.token)
    );
    assert_eq!(
        control.control(),
        (fixture.task.ident.id, Some(fixture.token))
    );
    let leaf_after = pend::locate(&fixture.task, leaf).unwrap();
    assert_eq!(leaf_after.sire(), Some(fixture.token));
    assert_eq!(leaf_after.permission(), leaf_permission);
    assert!(Arc::ptr_eq(leaf_after.grant(), &leaf_grant));
    drop(leaf_after);
    let child_after = pend::locate(&fixture.task, child).unwrap();
    assert!(child_after.sire().is_none());
    assert_eq!(child_after.permission(), child_permission);
    drop(child_after);
    assert!(fixture.team.space.views.registered(id).is_some());
    assert!(!fixture.team.space.nest.settled(id));

    restore_shootdown(asid);
    fixture.team.space.nest.retry_invalidation().unwrap();
    fixture.team.space.reap_views();
    assert!(fixture.team.space.views.registered(id).is_none());
    assert!(fixture.authorization.owns_geometry(&geometry));
    pend::release(&fixture.task, child).unwrap();
    assert_eq!(fixture.authorization.derivation_count(), 0);

    drop(child_meta);
    drop(child_source);
    drop(control);
    drop(leaf_grant);
    drop(geometry);
    fixture.finish();
    baseline.expect();
}

pub fn forget_requires_a_qualified_parent() {
    let baseline = Baseline::new();
    let fixture = Fixture::shared(4);
    let middle = branch(&fixture, fixture.token, 0, 3);
    let leaf = branch(&fixture, middle, 2, 1);
    let id = map(&fixture, middle);
    let child = derive(&fixture, middle);
    let (child_meta, child_source) = fixture.child(child);
    let (_, geometry) = child_source.lineage().unwrap();
    pend::reduce(&fixture.task, fixture.token, Permission::FETCH).unwrap();
    assert_eq!(
        pend::locate(&fixture.task, leaf).unwrap().permission(),
        Permission::FETCH | Permission::VEST,
        "Narrow must not retroactively narrow a descendant"
    );
    assert_eq!(
        pend::forget(&fixture.task, middle),
        Err(PieFail::Denied),
        "the replacement parent must cover every reparented authorization"
    );
    assert!(pend::locate(&fixture.task, middle).is_some());
    assert_eq!(
        pend::locate(&fixture.task, leaf).unwrap().sire(),
        Some(middle)
    );
    assert_eq!(
        fixture.authorization.derivation_token(&geometry),
        Some(middle)
    );
    assert_eq!(
        fixture
            .team
            .space
            .views
            .locate(id)
            .unwrap()
            .control
            .unwrap()
            .control(),
        (fixture.task.ident.id, Some(middle))
    );

    drop(child_meta);
    drop(child_source);
    drop(geometry);
    fixture.finish();
    baseline.expect();
}
