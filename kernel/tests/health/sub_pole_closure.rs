use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{MemoryCall, PieToken, PoleCall, PoleFail};

use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::nest::identity::{fail_shootdown, restore_shootdown};
use crate::core::work::room::messenger;
use crate::core::work::unit::team;
use crate::entry::call::pole::create_pole;

use super::baseline::Baseline;
use super::fixture::{self, Fixture};
use super::{flat_reference, pend, pole, rw, space};

pub fn parent_seal_retries_a_pending_descendant() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(4);
    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .unwrap();
    let (child_meta, child_source) = fixture.child(child);
    let reference = fixture.acquire(child, 0, 1, PieToken::NONE);
    assert!(reference > 0);
    let grandchild = create_pole(&fixture.task, flat_reference(reference), false).unwrap();
    assert_eq!(
        fixture
            .call(MemoryCall::Release {
                reference: flat_reference(reference),
            })
            .0,
        0
    );
    let (grandchild_meta, grandchild_source) = fixture.child(grandchild);
    let target = space();
    let (address, _) = pole::open(&grandchild_meta, grandchild, &target, rw()).unwrap();
    let view = target
        .nest
        .probe(crate::core::memory::address::VirtAddr::wrap(address))
        .unwrap();
    let id = view.identity();
    drop(view);
    pole::shut(&fixture.meta, fixture.token).unwrap();
    assert!(!fixture.meta.has_maps());
    assert!(!child_meta.has_maps());
    assert_eq!(fixture.authorization.binding_count(), 0);
    assert_eq!(grandchild_source.binding_count(), 1);

    let asid = target.nest.asid();
    fail_shootdown(asid);
    let seal = PoleCall::Seal {
        token: fixture.token,
    };
    assert_eq!(
        fixture::pole_call(&fixture.task, seal).0,
        PoleFail::Busy.code(),
        "a parent without local maps must still report its child's pending CPU cleanup"
    );
    assert!(!fixture.meta.alive());
    assert!(!child_meta.alive());
    assert!(!grandchild_meta.alive());
    assert_eq!(fixture.authorization.derivation_count(), 1);
    assert_eq!(child_source.derivation_count(), 1);
    assert!(target.views.registered(id).is_some());
    assert!(!target.nest.settled(id));
    assert!(!target.nest.retired());
    assert_eq!(
        fixture::pole_call(&fixture.task, seal).0,
        PoleFail::Busy.code(),
        "the original token must remain usable for cleanup retries"
    );

    restore_shootdown(asid);
    assert_eq!(fixture::pole_call(&fixture.task, seal).0, 0);
    assert!(target.views.registered(id).is_none());
    assert!(target.nest.settled(id));
    assert!(!target.nest.retired());
    assert_eq!(grandchild_source.binding_count(), 0);
    assert_eq!(child_source.derivation_count(), 0);
    assert_eq!(fixture.authorization.derivation_count(), 0);
    assert!(!pend::locate(&fixture.task, child).unwrap().alive());
    assert!(!pend::locate(&fixture.task, grandchild).unwrap().alive());
    target.retire().unwrap();
    drop(target);
    drop(grandchild_meta);
    drop(grandchild_source);
    drop(child_meta);
    drop(child_source);
    fixture.finish();
    baseline.expect();
}

pub fn parent_seal_closes_a_consumed_child() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);
    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .unwrap();
    let entry_at = VirtAddr::wrap(0x6500_0000);
    let data_at = VirtAddr::wrap(0x6600_0000);
    let target = fixture.target(entry_at);
    let reference = fixture.acquire(child, 0, 1, PieToken::NONE);
    assert!(reference > 0);
    let (id, _) = fixture.attach_to(reference, 0, 1, data_at, target.id, rw());
    assert!(id > 0);
    let id = id as usize;
    assert_eq!(
        fixture
            .call(MemoryCall::Release {
                reference: flat_reference(reference),
            })
            .0,
        0
    );
    let receiver = team::spawn(
        &target,
        Some(&fixture.task),
        entry_at.as_usize(),
        Vec::new(),
        0,
    )
    .unwrap();
    let source = target.space.views.locate(id).unwrap().source.clone();
    assert!(
        source.root().is_none(),
        "the delivered Pole metadata is gone"
    );
    let weak_source = Arc::downgrade(&source);
    drop(source);
    assert!(pend::locate(&fixture.task, child).is_none());
    pole::shut(&fixture.meta, fixture.token).unwrap();
    assert!(!fixture.meta.has_maps());
    let seal = PoleCall::Seal {
        token: fixture.token,
    };

    let asid = target.space.nest.asid();
    fail_shootdown(asid);
    assert_eq!(
        fixture::pole_call(&fixture.task, seal).0,
        PoleFail::Busy.code()
    );
    assert!(weak_source.upgrade().is_some());
    assert_eq!(fixture.authorization.derivation_count(), 1);
    assert!(target.space.views.registered(id).is_some());
    assert!(!target.space.nest.retired());
    restore_shootdown(asid);
    assert_eq!(fixture::pole_call(&fixture.task, seal).0, 0);
    assert!(weak_source.upgrade().is_none());
    assert_eq!(fixture.authorization.derivation_count(), 0);
    assert!(target.space.views.registered(id).is_none());
    assert!(target.space.nest.translate(data_at).is_none());
    assert!(!target.space.nest.retired());
    assert!(
        receiver.held(),
        "resource closure does not terminate the receiver"
    );
    assert!(!receiver.holds_control(id));

    messenger::slay(&receiver);
    messenger::bury();
    drop(receiver);
    drop(target);
    fixture.finish();
    baseline.expect();
}

pub fn sealed_source_keeps_its_inflight_allocation() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);
    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .unwrap();
    let (meta, source) = fixture.child(child);
    let flight = (1usize << (usize::BITS - 1)) | 23;
    {
        let _commit = crate::core::work::commit();
        source
            .enter_publish_range(
                flight,
                pend::Geometry::new(PageOffset::ZERO, PageCount::new(1).unwrap()).unwrap(),
            )
            .unwrap();
    }
    let seal = PoleCall::Seal { token: child };
    assert_eq!(
        fixture::pole_call(&fixture.task, seal).0,
        PoleFail::Busy.code()
    );
    assert!(!meta.alive());
    assert!(source.pending_close());
    assert_eq!(
        fixture.derive(PageOffset::new(1), PageCount::new(1).unwrap(), false),
        Err(env::PieFail::Busy),
        "resource invalidation does not settle an in-flight publication"
    );
    assert_eq!(fixture.authorization.derivation_count(), 1);
    assert_eq!(
        fixture::pole_call(&fixture.task, seal).0,
        PoleFail::Busy.code()
    );
    source.leave_publish(flight);
    assert!(!source.pending_close());
    assert_eq!(fixture.authorization.derivation_count(), 0);
    fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .unwrap();
    drop(meta);
    drop(source);
    fixture.finish();
    baseline.expect();
}

pub fn parent_seal_retries_a_failed_open() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);
    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .unwrap();
    let (meta, source) = fixture.child(child);
    let target = space();
    let asid = target.nest.asid();
    fail_shootdown(asid);
    assert_eq!(
        pole::open(&meta, child, &target, rw()),
        Err(env::PieFail::Busy)
    );
    assert!(!meta.has_maps());
    assert_eq!(source.binding_count(), 1);
    let binding = source.bindings().unwrap().pop().unwrap();
    let id = binding.id();
    assert!(target.views.registered(id).unwrap().view.is_none());
    pole::shut(&fixture.meta, fixture.token).unwrap();
    assert!(!fixture.meta.has_maps());
    let seal = PoleCall::Seal {
        token: fixture.token,
    };
    assert_eq!(
        fixture::pole_call(&fixture.task, seal).0,
        PoleFail::Busy.code()
    );
    assert!(!meta.alive());
    assert!(target.views.registered(id).is_some());
    assert_eq!(fixture.authorization.derivation_count(), 1);
    restore_shootdown(asid);
    assert_eq!(fixture::pole_call(&fixture.task, seal).0, 0);
    assert!(target.views.registered(id).is_none());
    assert_eq!(source.binding_count(), 0);
    assert_eq!(fixture.authorization.derivation_count(), 0);
    assert!(!target.nest.retired());
    target.retire().unwrap();
    drop(target);
    drop(binding);
    drop(meta);
    drop(source);
    fixture.finish();
    baseline.expect();
}
