use alloc::rc::Rc;
use core::cell::RefCell;

use env::{PieToken, TaskId};
use hub_api::coordination::{Prepare, Release, Wire, PREPARE, RELEASE};
use hub_api::{BindingState, Failure, PARTY_PCI};

use crate::driver::recovery::coord::Coordinator;
use crate::driver::recovery::operation::Operation;
use crate::driver::recovery::stage::Stage;
use crate::fixtures::action::Action;
use crate::fixtures::core::Core;
use crate::fixtures::handoff;
use crate::fixtures::peer::Peer;
use crate::fixtures::script::Script;

const HUB: TaskId = TaskId::new(1);
const EXEC: TaskId = TaskId::new(9);

fn setup() -> (Coordinator, Rc<RefCell<Script>>) {
    handoff::reset();
    let script = Rc::new(RefCell::new(Script::new()));
    let coord = Coordinator::new(
        HUB,
        Peer::new(script.clone()),
        Peer::new(script.clone()),
        Core::new(script.clone()),
        crate::profile::Profile { key: 7, requester: 3, bar: 0, caps: (0x50, 0x60, 0, 0) },
    );
    (coord, script)
}

fn prepare(serial: u64, execution: TaskId, key: u64) -> Wire {
    Wire::Prepare(Prepare {
        op: PREPARE,
        hub: HUB,
        serial,
        key,
        execution,
        back: PieToken::mint(1),
    })
}

fn release(serial: u64, execution: TaskId, reason: u8) -> Wire {
    Wire::Release(Release {
        op: RELEASE,
        hub: HUB,
        serial,
        key: 7,
        execution,
        reason,
        back: PieToken::mint(1),
    })
}

fn op(coord: &Coordinator) -> &Operation {
    coord.operation.as_ref().unwrap()
}

fn drive_release(coord: &mut Coordinator, request: Wire) -> hub_api::DmaSaid {
    for _ in 0..16 {
        let previous = coord.operation.as_ref().map(|op| op.stage);
        let said = coord.turn(HUB, request);
        let stage = coord.operation.as_ref().map(|op| op.stage);
        if said.status != wire::OK || stage == Some(Stage::Done) || stage == previous {
            return said;
        }
    }
    panic!("cleanup made too many distinct advances");
}

#[test]
fn successful_prepare_then_release_uses_exact_order() {
    let (mut coord, script) = setup();
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.state, BindingState::Active.code());
    assert_eq!(op(&coord).stage, Stage::Active);
    assert_eq!(op(&coord).iova, 0x2_0000_0000);
    let crate::pci_api::Wire::Place(place) = script.borrow().pci[0] else {
        panic!("exact BAR placement");
    };
    assert_eq!(place.requester, 3, "opaque device key 7 is not a requester");
    assert_eq!(place.bar, 0);
    let crate::iommu_api::Wire::Acquire(acquire) = script.borrow().iommu[0] else {
        panic!("IOMMU acquisition");
    };
    assert_eq!(acquire.requester, 3);
    assert_eq!(handoff::gives(), 1);
    assert_eq!(
        script.borrow().actions,
        [
            Action::PciPlace,
            Action::CoreAttach,
            Action::IommuAcquire,
            Action::IommuPublish,
            Action::PciEnable,
        ]
    );

    let said = drive_release(&mut coord, release(5, EXEC, 1));
    assert_eq!(said.state, BindingState::Closed.code());
    assert_eq!(op(&coord).stage, Stage::Done);
    assert_eq!(
        script.borrow().actions[5..],
        [
            Action::IommuInvalidate,
            Action::PciDisable,
            Action::CoreStop,
            Action::IommuQuiesce,
            Action::IommuRelease,
            Action::CoreDetach,
            Action::PciRelease,
        ]
    );
}

#[test]
fn lost_place_ack_then_release_still_runs_all_cleanup() {
    let (mut coord, script) = setup();
    script.borrow_mut().place_ok = false;
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(op(&coord).stage, Stage::Place);
    assert_eq!(op(&coord).bar, None);

    drive_release(&mut coord, release(5, EXEC, 2));
    assert_eq!(op(&coord).stage, Stage::Done);
    let actions = &script.borrow().actions;
    for expected in [
        Action::IommuInvalidate,
        Action::PciDisable,
        Action::CoreStop,
        Action::IommuQuiesce,
        Action::IommuRelease,
        Action::CoreDetach,
        Action::PciRelease,
    ] {
        assert!(actions.contains(&expected), "missing {expected:?}");
    }
}

#[test]
fn lost_acquire_ack_then_release_runs_iommu_cleanup_and_keeps_binding() {
    let (mut coord, script) = setup();
    script.borrow_mut().acquire_ok = false;
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(op(&coord).stage, Stage::Acquire);
    assert_eq!(op(&coord).serial, 5);

    drive_release(&mut coord, release(5, EXEC, 2));
    assert_eq!(op(&coord).serial, 5);
    assert_eq!(op(&coord).stage, Stage::Done);
    let actions = &script.borrow().actions;
    assert!(actions.contains(&Action::IommuInvalidate));
    assert!(actions.contains(&Action::IommuRelease));
    assert!(actions.contains(&Action::CoreStop));
}

#[test]
fn attach_error_after_core_owns_map_still_stops_and_detaches() {
    let (mut coord, script) = setup();
    script.borrow_mut().attach_ok = false;
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(op(&coord).stage, Stage::Attach);
    assert!(op(&coord).bar.is_some());

    drive_release(&mut coord, release(5, EXEC, 2));
    let actions = &script.borrow().actions;
    assert!(actions.contains(&Action::CoreStop));
    assert!(actions.contains(&Action::CoreDetach));
}

#[test]
fn failed_handoff_stays_handoff_and_never_re_enables() {
    let (mut coord, script) = setup();
    handoff::fail();
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(op(&coord).stage, Stage::Handoff);
    assert_eq!(
        script
            .borrow()
            .actions
            .iter()
            .filter(|action| **action == Action::PciEnable)
            .count(),
        1
    );

    handoff::reset();
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.state, BindingState::Active.code());
    assert_eq!(op(&coord).stage, Stage::Active);
    assert_eq!(
        script
            .borrow()
            .actions
            .iter()
            .filter(|action| **action == Action::PciEnable)
            .count(),
        1
    );
}

#[test]
fn release_before_prepare_closes_and_blocks_late_prepare() {
    let (mut coord, script) = setup();
    drive_release(&mut coord, release(5, EXEC, 2));
    assert_eq!(op(&coord).stage, Stage::Done);
    let before = script.borrow().actions.len();
    let said = coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
    assert_eq!(script.borrow().actions.len(), before);
    assert!(!script.borrow().actions.contains(&Action::PciPlace));
}

#[test]
fn foreign_sender_wrong_hub_and_wrong_binding_cannot_advance() {
    let (mut coord, script) = setup();
    let said = coord.turn(TaskId::new(2), prepare(5, EXEC, 7));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Denied)));
    assert!(script.borrow().actions.is_empty());

    let mut wrong_hub = prepare(5, EXEC, 7);
    if let Wire::Prepare(frame) = &mut wrong_hub {
        frame.hub = TaskId::new(2);
    }
    let said = coord.turn(HUB, wrong_hub);
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Denied)));
    assert!(script.borrow().actions.is_empty());

    coord.turn(HUB, prepare(5, EXEC, 7));
    let said = coord.turn(HUB, release(6, EXEC, 2));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
    let said = coord.turn(HUB, release(5, TaskId::new(11), 2));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
}

#[test]
fn first_failure_is_immutable_after_a_different_failure() {
    let (mut coord, script) = setup();
    script.borrow_mut().place_ok = false;
    coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(op(&coord).first_failure, Some(Failure { party: PARTY_PCI, op: 1, code: 0 }));

    {
        let mut script = script.borrow_mut();
        script.place_ok = true;
        script.place_status = 9;
    }
    coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(op(&coord).first_failure, Some(Failure { party: PARTY_PCI, op: 1, code: 0 }));
}

#[test]
fn retry_reuses_retained_bar_and_does_not_place_again() {
    let (mut coord, script) = setup();
    script.borrow_mut().attach_ok = false;
    coord.turn(HUB, prepare(5, EXEC, 7));
    let bar = op(&coord).bar;
    assert!(bar.is_some());
    assert_eq!(
        script.borrow().actions.iter().filter(|a| **a == Action::PciPlace).count(),
        1
    );

    script.borrow_mut().attach_ok = true;
    coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(
        script.borrow().actions.iter().filter(|a| **a == Action::PciPlace).count(),
        1
    );
    assert_eq!(
        script.borrow().actions.iter().filter(|a| **a == Action::CoreAttach).count(),
        2
    );
    assert_eq!(op(&coord).bar, bar);
    assert_eq!(op(&coord).stage, Stage::Active);
}

#[test]
fn failed_push_reuses_the_same_writer_without_reenabling() {
    let (mut coord, script) = setup();
    handoff::push_ok(false);
    coord.turn(HUB, prepare(5, EXEC, 7));
    let writer = op(&coord).handoff;
    assert!(writer.is_some());
    assert_eq!(op(&coord).stage, Stage::Handoff);
    coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(op(&coord).handoff, writer);
    assert_eq!(handoff::gives(), 1);
    assert_eq!(script.borrow().actions.iter().filter(|action| **action == Action::PciEnable).count(), 1);
    handoff::push_ok(true);
    coord.turn(HUB, prepare(5, EXEC, 7));
    assert_eq!(op(&coord).stage, Stage::Active);
    assert_eq!(handoff::gives(), 1);
}

#[test]
fn failed_writer_reclamation_retains_its_handle_and_blocks_closed_reply() {
    let (mut coord, script) = setup();
    coord.turn(HUB, prepare(5, EXEC, 7));
    let writer = op(&coord).handoff;
    handoff::release_ok(false);
    let said = drive_release(&mut coord, release(5, EXEC, 2));
    assert_eq!(said.state, BindingState::Closing.code());
    assert_eq!(op(&coord).stage, Stage::CloseHandoff);
    assert_eq!(op(&coord).handoff, writer);
    assert!(!script.borrow().actions.contains(&Action::PciRelease));
    assert_eq!(handoff::releases(), 0);
    handoff::release_ok(true);
    drive_release(&mut coord, release(5, EXEC, 2));
    assert_eq!(op(&coord).handoff, None);
    assert_eq!(op(&coord).stage, Stage::Done);
    assert_eq!(handoff::releases(), 1);
}

#[test]
fn cached_bar_responsibility_survives_closed_binding_and_the_next_prepare() {
    let (mut coord, script) = setup();
    coord.turn(HUB, prepare(5, EXEC, 7));
    let bar = op(&coord).bar;
    drive_release(&mut coord, release(5, EXEC, 1));
    assert_eq!(op(&coord).bar, bar);
    script.borrow_mut().place_ok = false;
    coord.turn(HUB, prepare(6, EXEC, 7));
    assert_eq!(op(&coord).stage, Stage::Place);
    assert_eq!(op(&coord).bar, bar);
}

#[test]
fn a_foreign_device_key_or_invalid_reason_cannot_create_a_cleanup_record() {
    let (mut coord, script) = setup();
    let mut request = release(5, EXEC, 2);
    if let Wire::Release(frame) = &mut request { frame.key = 8; }
    assert_ne!(coord.turn(HUB, request).status, wire::OK);
    assert!(coord.operation.is_none());
    assert!(script.borrow().actions.is_empty());
    assert_ne!(coord.turn(HUB, release(5, EXEC, 0)).status, wire::OK);
    assert!(coord.operation.is_none());
    assert!(script.borrow().actions.is_empty());
    coord.turn(HUB, prepare(5, EXEC, 7));
    let count = script.borrow().actions.len();
    assert_ne!(coord.turn(HUB, release(5, EXEC, 0)).status, wire::OK);
    assert_eq!(op(&coord).stage, Stage::Active);
    assert_eq!(op(&coord).reason, hub_api::Close::Normal);
    assert_eq!(script.borrow().actions.len(), count);
}

#[test]
fn each_release_yields_after_one_effect_and_retains_the_next_stage() {
    let (mut coord, script) = setup();
    coord.turn(HUB, prepare(5, EXEC, 7));
    let count = script.borrow().actions.len();
    let said = coord.turn(HUB, release(5, EXEC, 1));
    assert_eq!(said.state, BindingState::Closing.code());
    assert_eq!(op(&coord).stage, Stage::Disable);
    assert_eq!(script.borrow().actions[count..], [Action::IommuInvalidate]);
    coord.turn(HUB, release(5, EXEC, 1));
    assert_eq!(op(&coord).stage, Stage::Stop);
    assert_eq!(script.borrow().actions[count..], [Action::IommuInvalidate, Action::PciDisable]);
    coord.turn(HUB, release(5, EXEC, 1));
    assert_eq!(op(&coord).stage, Stage::Quiesce);
    assert_eq!(script.borrow().actions[count..],
        [Action::IommuInvalidate, Action::PciDisable, Action::CoreStop]);
}
