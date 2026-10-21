mod boundary;
mod effects;
mod failure;

use env::{PieToken, TaskId};
use hub_api::{Enroll, Fail, INVENTORY_MAX};
use crate::hub_core::{ClaimRequest, Ledger, Owner};
use boundary::Boundary;
use effects::Effects;
use failure::Failure;

fn request(requester: u32) -> Enroll {
    Enroll {
        op: hub_api::DMA_ENROLL,
        requester,
        line: 7,
        name: format!("device-{requester}"),
        class: "virtio-blk".into(),
        back: PieToken::NONE,
    }
}

fn prepare(ledger: &mut Ledger, requester: u32) {
    ledger.prepare_enrollment(TaskId::new(2), request(requester)).unwrap();
}

pub(super) fn enroll(ledger: &mut Ledger, requesters: &[u32]) {
    let mut effects = Effects::default();
    for &requester in requesters {
        prepare(ledger, requester);
        assert_eq!(ledger.advance_enrollment(requester, &mut effects), Some(Ok(())));
    }
}

fn hidden(ledger: &mut Ledger) {
    assert!(ledger.device(8).is_none());
    assert!(ledger.device_by_door(PieToken::mint(100)).is_none());
    assert_eq!(ledger.doors("virtio-blk".into()).count(), 0);
    assert_eq!(ledger.list("virtio-blk".into(), 0).n, 0);
    assert_eq!(ledger.claim(ClaimRequest {
        from: TaskId::new(9), id: 1, requester: 8,
        sensor: PieToken::mint(9), recovery: TaskId::new(3),
    }), Err(Fail::Unknown));
    assert!(matches!(ledger.claim_with(
        PieToken::mint(100),
        Owner { task: TaskId::new(9), sensor: PieToken::mint(9) },
        |_| true,
        |_| -> Result<(), Fail> { panic!("unpublished resource was delivered") },
    ), Err(Fail::Unknown)));
}

fn complete(ledger: &mut Ledger, effects: &mut Effects) {
    prepare(ledger, 8);
    assert_eq!(ledger.advance_enrollment(8, effects), Some(Ok(())));
    assert!(ledger.device(8).is_some());
    assert_eq!(effects.coalitions.len(), 1);
    assert_eq!(effects.doors.len(), 1);
    assert_eq!(effects.parts.len(), 1);
    assert_eq!(effects.bindings.len(), 1);
    assert_eq!(ledger.doors("virtio-blk".into()).count(), 1);
}

#[test]
fn every_registration_boundary_retains_completed_effects_and_hides_pending_devices() {
    let boundaries = [
        Boundary::Found, Boundary::Admit, Boundary::Activate,
        Boundary::Door, Boundary::Part, Boundary::Bind,
    ];
    for boundary in boundaries {
        let mut ledger = Ledger::new();
        let mut effects = Effects { failure: Some(Failure::Before(boundary)), ..Effects::default() };
        prepare(&mut ledger, 8);
        assert_eq!(ledger.advance_enrollment(8, &mut effects), Some(Err(Failure::Before(boundary))));
        hidden(&mut ledger);
        if matches!(boundary, Boundary::Found | Boundary::Admit | Boundary::Activate) {
            assert!(ledger.coalition_of("virtio-blk".into()).is_none());
        }
        complete(&mut ledger, &mut effects);
        let calls = effects.calls.clone();
        complete(&mut ledger, &mut effects);
        assert_eq!(effects.calls, calls, "confirmed enrollment must not publish again");
        for completed in boundaries.into_iter().take_while(|step| *step != boundary) {
            assert_eq!(effects.calls.iter().filter(|step| **step == completed).count(), 1);
        }
    }
}

#[test]
fn lost_replies_reuse_the_same_creation_directory_operation_and_door() {
    for boundary in [Boundary::Found, Boundary::Admit, Boundary::Activate, Boundary::Part, Boundary::Bind] {
        let mut ledger = Ledger::new();
        let mut effects = Effects { failure: Some(Failure::Reply(boundary)), ..Effects::default() };
        prepare(&mut ledger, 8);
        assert_eq!(ledger.advance_enrollment(8, &mut effects), Some(Err(Failure::Reply(boundary))));
        hidden(&mut ledger);
        complete(&mut ledger, &mut effects);
        assert_eq!(effects.calls.iter().filter(|step| **step == boundary).count(), 2);
        assert_eq!(effects.admitted.len(), 1);
        assert_eq!(effects.activated.len(), 1);
    }
}

#[test]
fn repeated_unknown_bind_replies_never_mint_another_door_or_report_ready() {
    let mut ledger = Ledger::new();
    let mut effects = Effects::default();
    for _ in 0..8 {
        effects.failure = Some(Failure::Reply(Boundary::Bind));
        prepare(&mut ledger, 8);
        assert_eq!(ledger.advance_enrollment(8, &mut effects), Some(Err(Failure::Reply(Boundary::Bind))));
        hidden(&mut ledger);
        assert_eq!(effects.doors.len(), 1);
        assert_eq!(effects.bindings.len(), 1);
    }
    complete(&mut ledger, &mut effects);
}

#[test]
fn conflicting_replays_cannot_change_pending_or_published_registration() {
    for pending in [true, false] {
        let mut ledger = Ledger::new();
        let mut effects = Effects::default();
        prepare(&mut ledger, 8);
        if pending {
            effects.failure = Some(Failure::Reply(Boundary::Bind));
        }
        let _ = ledger.advance_enrollment(8, &mut effects);
        for changed in 0..4 {
            let mut enroll = request(8);
            let from = if changed == 0 { TaskId::new(99) } else { TaskId::new(2) };
            match changed {
                1 => enroll.name = "other".into(),
                2 => enroll.class = "other".into(),
                3 => enroll.line += 1,
                _ => {}
            }
            assert_eq!(ledger.prepare_enrollment(from, enroll), Err(Fail::Conflict));
        }
        complete(&mut ledger, &mut effects);
        assert_eq!(ledger.count(), 1);
    }
}

#[test]
fn pending_devices_share_the_league_and_publish_in_reservation_order() {
    let mut ledger = Ledger::new();
    let mut effects = Effects { failure: Some(Failure::Reply(Boundary::Found)), ..Effects::default() };
    prepare(&mut ledger, 8);
    assert_eq!(ledger.advance_enrollment(8, &mut effects), Some(Err(Failure::Reply(Boundary::Found))));
    prepare(&mut ledger, 9);
    let calls = effects.calls.clone();
    assert_eq!(ledger.advance_enrollment(9, &mut effects), None);
    assert_eq!(effects.calls, calls);
    assert_eq!(ledger.advance_enrollment(8, &mut effects), Some(Ok(())));
    assert_eq!(ledger.advance_enrollment(9, &mut effects), Some(Ok(())));
    assert_eq!(effects.coalitions.len(), 1);
    assert_eq!(effects.admitted.len(), 1);
    assert_eq!(effects.activated.len(), 1);
    assert_eq!(effects.doors.len(), 2);
    assert_eq!(effects.bindings.len(), 2);
    let mut other = request(10);
    other.class = "other".into();
    ledger.prepare_enrollment(TaskId::new(2), other).unwrap();
    assert_eq!(ledger.advance_enrollment(10, &mut effects), Some(Ok(())));
    assert_eq!(effects.coalitions.len(), 2);
    assert_ne!(effects.coalitions[0].0, effects.coalitions[1].0);
}

#[test]
fn requester_order_cannot_reorder_operator_ids_or_abandon_an_unknown_receipt() {
    for boundary in [Boundary::Part, Boundary::Bind] {
        let mut ledger = Ledger::new();
        let mut effects = Effects { failure: Some(Failure::Reply(boundary)), ..Effects::default() };
        prepare(&mut ledger, 500);
        prepare(&mut ledger, 1);
        assert_eq!(ledger.advance_enrollment(500, &mut effects), Some(Err(Failure::Reply(boundary))));
        let calls = effects.calls.clone();
        assert_eq!(ledger.advance_enrollment(1, &mut effects), None);
        assert_eq!(effects.calls, calls);
        assert!(ledger.device(1).is_none());
        assert_eq!(ledger.advance_enrollment(500, &mut effects), Some(Ok(())));
        assert_eq!(ledger.advance_enrollment(1, &mut effects), Some(Ok(())));
        assert_eq!(effects.parts.len(), 2);
        assert_eq!(effects.bindings.len(), 2);
        assert!(effects.parts[0].0.0 < effects.bindings[0].0.0);
        assert!(effects.bindings[0].0.0 < effects.parts[1].0.0);
        assert!(effects.parts[1].0.0 < effects.bindings[1].0.0);
    }
}

#[test]
fn capacity_names_and_settlement_are_checked_before_acquiring_resources() {
    let mut ledger = Ledger::new();
    let mut invalid = request(8);
    invalid.class = "../bad".into();
    assert_eq!(ledger.prepare_enrollment(TaskId::new(2), invalid), Err(Fail::Bad));
    assert_eq!(ledger.count(), 0);
    for requester in 0..INVENTORY_MAX as u32 {
        prepare(&mut ledger, requester);
    }
    assert_eq!(ledger.count(), INVENTORY_MAX);
    assert_eq!(ledger.prepare_enrollment(TaskId::new(2), request(INVENTORY_MAX as u32)), Err(Fail::Full));
    let mut duplicate = request(INVENTORY_MAX as u32);
    duplicate.name = "device-0".into();
    assert_eq!(ledger.prepare_enrollment(TaskId::new(2), duplicate), Err(Fail::Conflict));
    prepare(&mut ledger, 0);
    let mut effects = Effects::default();
    ledger.settle();
    assert_eq!(ledger.prepare_enrollment(TaskId::new(2), request(0)), Err(Fail::Denied));
    assert_eq!(ledger.advance_enrollment(0, &mut effects), None);
    assert!(effects.calls.is_empty());
}

#[test]
fn machine_entries_have_no_pci_identity_and_claim_transitions_preserve_device_origin() {
    use hub_api::DeviceId;
    use crate::hub_core::Entry;

    let mut ledger = Ledger::new();
    let machine = Entry {
        name: "machine".into(), class: "virtio-blk".into(), line: 0,
        page: PieToken::mint(20), door: PieToken::mint(21),
    };
    ledger.enroll(machine.clone()).unwrap();
    assert!(ledger.device(0).is_none());
    assert!(ledger.device_by_door(machine.door).is_none());
    assert_eq!(ledger.devices().count(), 0);

    enroll(&mut ledger, &[0]);
    let device = DeviceId { pci: TaskId::new(2), requester: 0 };
    let door = ledger.doors("virtio-blk".into())
        .find(|(name, _)| name.as_str() == "device-0").unwrap().1;
    let owner = Owner { task: TaskId::new(9), sensor: PieToken::mint(9) };
    assert_eq!(ledger.device(0), Some(device));
    assert_eq!(ledger.device_by_door(door), Some(device));
    assert_eq!(ledger.devices().collect::<Vec<_>>(), vec![device]);
    assert!(matches!(ledger.claim_with(door, owner.clone(), |_| false,
        |_| -> Result<(), Fail> { Err(Fail::Full) }), Err(Fail::Full)));
    assert_eq!(ledger.device_by_door(door), Some(device));
    let (entry, ()) = ledger.claim_with(door, owner, |_| false, |_| Ok(())).unwrap();
    assert_eq!(entry.name, "device-0");
    assert_eq!(ledger.device_by_door(door), Some(device));
    assert_eq!(ledger.list("virtio-blk".into(), 0).held[0], 0b10);
    assert_eq!(ledger.vacate(|_| false), 1);
    assert_eq!(ledger.device_by_door(door), Some(device));
    assert_eq!(ledger.list("virtio-blk".into(), 0).held[0], 0);
    assert_eq!(ledger.count(), 2);
}
