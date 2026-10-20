use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{AcquireSource, MemoryCall, MemoryFail, PieFail, PieToken, TeamId, UnitFail, ViewId};

use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::nest::Access;
use crate::core::memory::nest::identity::{fail_shootdown, restore_shootdown};
use crate::core::work::room::{messenger, scheduler};
use crate::core::work::unit::team;

use super::baseline::Baseline;
use super::fixture::{self, Fixture};
use super::{flat_reference, pend};

const ENTRY: VirtAddr = VirtAddr::wrap(0x6500_0000);
const DATA: VirtAddr = VirtAddr::wrap(0x6600_0000);

pub fn first_delivery_keeps_the_parent_allocation() {
    let baseline = Baseline::new();
    let husks = messenger::probe_bookkeeping().1;
    let fixture = Fixture::new(3);
    let token = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .expect("derived child");
    let (meta, source) = fixture.child(token);
    let weak_source = Arc::downgrade(&source);
    let (_, geometry) = source.lineage().expect("derived lineage");
    let target = fixture.target(ENTRY);
    let reference = fixture.acquire(token, 0, 1, PieToken::NONE);
    assert!(reference > 0);
    let (raw_id, address) = fixture.attach_to(
        reference,
        0,
        1,
        DATA,
        target.id,
        Access::READ | Access::WRITE,
    );
    assert!(
        raw_id > 0,
        "stage the derived resource through Memory Attach"
    );
    assert_eq!(address, DATA.as_usize());
    let id = raw_id as usize;
    let binding = target.space.views.locate(id).unwrap().binding.clone();
    assert_eq!(source.binding_count(), 1);
    assert_eq!(
        fixture
            .call(MemoryCall::Release {
                reference: flat_reference(reference),
            })
            .0,
        0
    );

    scheduler::roster::fail_next_reservation();
    assert!(matches!(
        team::spawn(
            &target,
            Some(&fixture.task),
            ENTRY.as_usize(),
            Vec::new(),
            0
        ),
        Err(UnitFail::OoM)
    ));
    assert!(!target.ready());
    assert_eq!(target.staged_len(), 1);
    assert!(pend::locate(&fixture.task, token).is_some());
    assert!(fixture.authorization.owns_geometry(&geometry));
    assert_eq!(source.binding_count(), 1);
    assert_eq!(
        target
            .space
            .views
            .locate(id)
            .unwrap()
            .control
            .unwrap()
            .control(),
        (fixture.task.ident.id, Some(token))
    );

    let receiver = team::spawn(
        &target,
        Some(&fixture.task),
        ENTRY.as_usize(),
        Vec::new(),
        0,
    )
    .unwrap();
    assert!(target.ready());
    assert_eq!(target.staged_len(), 0);
    assert!(pend::locate(&fixture.task, token).is_none());
    let entry = target.space.views.locate(id).unwrap();
    assert!(Arc::ptr_eq(&binding, &entry.binding));
    assert_eq!(
        entry.control.as_ref().unwrap().control(),
        (receiver.ident.id, None)
    );
    assert!(Arc::ptr_eq(&entry.source, &source));
    assert_eq!(source.binding_count(), 1);
    assert!(!source.private_owner(receiver.ident.id));
    drop(entry);
    drop(meta);
    drop(source);

    assert!(weak_source.upgrade().is_some());
    assert!(fixture.authorization.owns_geometry(&geometry));
    assert_eq!(
        fixture.derive(PageOffset::new(1), PageCount::new(1).unwrap(), false),
        Err(PieFail::Busy)
    );
    let view = ViewId::from_bytes(&(id as u64).to_le_bytes()).unwrap();
    assert_eq!(
        fixture::call(
            &receiver,
            MemoryCall::Acquire {
                source: AcquireSource::View {
                    team: TeamId::new(0),
                    view,
                    offset: 0,
                    count: 1,
                },
                retention: PieToken::NONE,
            },
        )
        .0,
        MemoryFail::Denied.code(),
        "delivery transfers control, not a new source authorization"
    );

    let asid = target.space.nest.asid();
    fail_shootdown(asid);
    messenger::slay(&receiver);
    messenger::bury();
    assert_eq!(messenger::probe_bookkeeping().1, husks + 1);
    assert!(target.space.views.registered(id).is_some());
    assert!(weak_source.upgrade().is_some());
    assert!(fixture.authorization.owns_geometry(&geometry));
    assert_eq!(
        fixture.derive(PageOffset::new(1), PageCount::new(1).unwrap(), false),
        Err(PieFail::Busy)
    );

    restore_shootdown(asid);
    messenger::bury();
    assert_eq!(messenger::probe_bookkeeping().1, husks);
    assert!(target.space.views.registered(id).is_none());
    assert!(weak_source.upgrade().is_none());
    assert_eq!(fixture.authorization.derivation_count(), 0);
    fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .expect("reuse after the delivered CPU obligation settles");

    drop(receiver);
    drop(target);
    drop(binding);
    drop(geometry);
    fixture.finish();
    baseline.expect();
}
