use alloc::rc::Rc;
use core::cell::RefCell;

use env::{PieToken, TaskId};
use hub_api::coordination::{Prepare, Release, Wire, PREPARE, RELEASE};
use hub_api::{BindingState, Failure, PARTY_PCI};

use crate::driver::recovery::coord::Coordinator;
use crate::driver::recovery::core::Core;
use crate::driver::recovery::operation::Operation;
use crate::driver::recovery::peers::Peers;
use crate::driver::recovery::profile::Profile;
use crate::driver::recovery::stage::Stage;
use crate::fixtures::action::Action;
use crate::fixtures::handoff;
use crate::fixtures::group::Group;
use crate::fixtures::peer::Peer;
use crate::fixtures::script::{self, Script};
use crate::fixtures::state;

const HUB: TaskId = TaskId::new(1);
const EXEC: TaskId = TaskId::new(9);
const ENGINE: TaskId = TaskId::new(10);
const KEY: u64 = 7;
const DOMAIN: u64 = 23;

struct Bench {
    coord: Coordinator,
    peers: Peers,
    script: Rc<RefCell<Script>>,
    stages: Vec<Stage>,
}

fn profile(key: u64, requester: u16) -> Profile {
    Profile { key, requester, bar: 0, caps: (0x50, 0x60, 0, 0) }
}

fn setup() -> Bench {
    handoff::reset();
    state::reset();
    let script = Rc::new(RefCell::new(Script::new()));
    script::install(&script);
    let peers = Peers {
        hub: HUB,
        pci: Peer::new(script.clone()),
        iommu: Peer::new(script.clone()),
    };
    let core = Core::new(peers.pci.host());
    Bench {
        coord: Coordinator::new(core, profile(KEY, 3), crate::domain_id::DomainId::new(ENGINE, DOMAIN)),
        peers,
        script,
        stages: Vec::new(),
    }
}

fn group() -> Group {
    handoff::reset();
    state::reset();
    let script = Rc::new(RefCell::new(Script::new()));
    script::install(&script);
    script.borrow_mut().iovas = vec![(23, 0x2_0000_0000), (24, 0x2_0020_0000)];
    let peers = Peers {
        hub: HUB,
        pci: Peer::new(script.clone()),
        iommu: Peer::new(script.clone()),
    };
    let coords = [(KEY, 3, 23u64), (8, 4, 24u64)]
        .into_iter()
        .map(|(key, requester, domain)| {
            let core = Core::new(peers.pci.host());
            Coordinator::new(core, profile(key, requester), crate::domain_id::DomainId::new(ENGINE, domain))
        })
        .collect();
    Group { coords, peers, script, stages: vec![Vec::new(), Vec::new()] }
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
        key: KEY,
        execution,
        reason,
        back: PieToken::mint(1),
    })
}

fn supply(serial: u64, source: usize) -> crate::client::Wire {
    crate::client::Wire::Supply(crate::client::Supply {
        op: crate::client::SUPPLY,
        hub: HUB,
        serial,
        source: PieToken::mint(source),
        access: env::Access::FETCH_STORE.bits().bits(),
        back: PieToken::mint(1),
    })
}

fn op(coord: &Coordinator) -> &Operation {
    coord.operation.as_ref().unwrap()
}

fn stage(coord: &Coordinator) -> Option<Stage> {
    coord.operation.as_ref().map(|op| op.stage)
}

fn progress(coord: &Coordinator) -> (Option<Stage>, bool, bool, bool) {
    let Some(op) = coord.operation.as_ref() else {
        return (None, false, false, false);
    };
    (Some(op.stage), op.exchange.is_some(), op.source.is_some(), op.client.is_some())
}

fn record(coord: &Coordinator, stages: &mut Vec<Stage>) {
    if let Some(stage) = stage(coord) {
        stages.push(stage);
    }
}

fn drive_prepare(
    coord: &mut Coordinator,
    peers: &Peers,
    serial: u64,
    source: usize,
    stop_on_failure: bool,
    stages: &mut Vec<Stage>,
) -> hub_api::DmaSaid {
    let request = prepare(serial, EXEC, coord.profile.key);
    let mut said = coord.turn(peers, HUB, request);
    for _ in 0..192 {
        if stage(coord) == Some(Stage::Active) {
            return said;
        }
        if stop_on_failure
            && op(coord).first_failure.is_some()
            && op(coord).exchange.is_none()
        {
            return said;
        }
        if stage(coord) == Some(Stage::Acquire) && op(coord).source.is_none() {
            let _ = coord.client(peers, EXEC, supply(serial, source));
        }
        let before = progress(coord);
        coord.advance(peers);
        record(coord, stages);
        said = coord.turn(peers, HUB, request);
        if stage(coord) == Some(Stage::Active) {
            return said;
        }
        let after = progress(coord);
        if after == before {
            let operation = op(coord);
            if operation.exchange.is_some() {
                return said;
            }
            if operation.stage == Stage::Stop {
                continue;
            }
            if stop_on_failure && operation.first_failure.is_some() {
                return said;
            }
            return said;
        }
    }
    panic!("prepare made too many advances");
}

fn drive_release(
    coord: &mut Coordinator,
    peers: &Peers,
    request: Wire,
    stop_on_failure: bool,
    stages: &mut Vec<Stage>,
) -> hub_api::DmaSaid {
    let mut said = coord.turn(peers, HUB, request);
    for _ in 0..192 {
        if stage(coord) == Some(Stage::Done) {
            return said;
        }
        let before = progress(coord);
        coord.advance(peers);
        record(coord, stages);
        said = coord.turn(peers, HUB, request);
        if stage(coord) == Some(Stage::Done) {
            return said;
        }
        let after = progress(coord);
        if after == before {
            let operation = op(coord);
            if operation.exchange.is_some() {
                return said;
            }
            if operation.stage == Stage::Stop {
                continue;
            }
            if stop_on_failure && operation.first_failure.is_some() {
                return said;
            }
            return said;
        }
    }
    panic!("cleanup made too many advances");
}

fn assert_order(stages: &[Stage], expected: &[Stage]) {
    let mut at = 0;
    for stage in stages {
        if expected.get(at) == Some(stage) {
            at += 1;
        }
    }
    assert_eq!(at, expected.len(), "missing stage order {expected:?} in {stages:?}");
}

impl Bench {
    fn turn(&mut self, wire: Wire) -> hub_api::DmaSaid {
        self.coord.turn(&self.peers, HUB, wire)
    }

    fn advance(&mut self) -> bool {
        let advanced = self.coord.advance(&self.peers);
        record(&self.coord, &mut self.stages);
        advanced
    }

    fn prepare(&mut self, serial: u64) -> hub_api::DmaSaid {
        drive_prepare(&mut self.coord, &self.peers, serial, 0xd1, true, &mut self.stages)
    }

    fn resume_prepare(&mut self, serial: u64) -> hub_api::DmaSaid {
        drive_prepare(&mut self.coord, &self.peers, serial, 0xd1, false, &mut self.stages)
    }

    fn release(&mut self) -> hub_api::DmaSaid {
        drive_release(&mut self.coord, &self.peers, release(5, EXEC, 2), true, &mut self.stages)
    }

    fn release_serial(&mut self, serial: u64, reason: u8) -> hub_api::DmaSaid {
        drive_release(
            &mut self.coord, &self.peers, release(serial, EXEC, reason), true, &mut self.stages,
        )
    }

    fn resume_release(&mut self, serial: u64) -> hub_api::DmaSaid {
        drive_release(
            &mut self.coord, &self.peers, release(serial, EXEC, 2), false, &mut self.stages,
        )
    }

    fn arrive_acquire(&mut self, serial: u64) -> hub_api::DmaSaid {
        let request = prepare(serial, EXEC, KEY);
        let mut said = self.turn(request);
        for _ in 0..32 {
            if stage(&self.coord) == Some(Stage::Acquire)
                && op(&self.coord).source.is_none()
            {
                return said;
            }
            self.advance();
            said = self.turn(request);
        }
        panic!("did not arrive at Acquire");
    }
}

impl Group {
    fn turn(&mut self, index: usize, wire: Wire) -> hub_api::DmaSaid {
        self.coords[index].turn(&self.peers, HUB, wire)
    }

    fn advance(&mut self, index: usize) -> bool {
        let advanced = self.coords[index].advance(&self.peers);
        record(&self.coords[index], &mut self.stages[index]);
        advanced
    }

    fn prepare(&mut self, index: usize, serial: u64) -> hub_api::DmaSaid {
        drive_prepare(
            &mut self.coords[index], &self.peers, serial, 0xd1 + index,
            true, &mut self.stages[index],
        )
    }

    fn arrive_acquire(&mut self, index: usize, serial: u64, source: usize) -> hub_api::DmaSaid {
        let request = prepare(serial, EXEC, self.coords[index].profile.key);
        let mut said = self.turn(index, request);
        for _ in 0..32 {
            if stage(&self.coords[index]) == Some(Stage::Acquire)
                && op(&self.coords[index]).source.is_none()
            {
                let _ = self.coords[index].client(&self.peers, EXEC, supply(serial, source));
                return said;
            }
            self.advance(index);
            said = self.turn(index, request);
        }
        panic!("did not arrive at Acquire");
    }

    fn resume_prepare(&mut self, index: usize, serial: u64) -> hub_api::DmaSaid {
        drive_prepare(
            &mut self.coords[index], &self.peers, serial, 0xd1 + index,
            false, &mut self.stages[index],
        )
    }

    fn release(&mut self, index: usize, serial: u64) -> hub_api::DmaSaid {
        drive_release(
            &mut self.coords[index], &self.peers, release(serial, EXEC, 2),
            true, &mut self.stages[index],
        )
    }
}

#[test]
fn successful_prepare_then_release_uses_exact_order() {
    let mut bench = setup();
    let said = bench.prepare(5);
    assert_eq!(said.state, BindingState::Active.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Active));
    assert_eq!(op(&bench.coord).iova, 0x2_0000_0000);
    let crate::pci_api::Wire::Place(place) = bench.script.borrow().pci[0] else {
        panic!("exact BAR placement");
    };
    assert_eq!(place.requester, 3, "opaque device key 7 is not a requester");
    assert_eq!(place.bar, 0);
    let crate::iommu_api::Wire::Acquire(acquire) = bench.script.borrow().iommu[0] else {
        panic!("IOMMU acquisition");
    };
    assert_eq!(acquire.engine, ENGINE);
    assert_eq!(acquire.key, DOMAIN, "domain key is neither device key nor requester");
    assert_eq!(acquire.hub, HUB);
    assert_eq!(acquire.serial, 5);
    assert_eq!(acquire.source, PieToken::mint(0xd1));
    assert_eq!(handoff::gives(), 1);
    let coords = handoff::last().unwrap();
    assert_eq!(coords.bar, PieToken::mint(9));
    assert_ne!(coords.bar, op(&bench.coord).bar.unwrap(), "Work receipt is not the Driver's PCI token");
    assert_eq!(coords.bar_pages, 1);
    assert_eq!((coords.iova, coords.bytes), (bench.script.borrow().iova, 6 * env::PAGE_SIZE as u64));
    assert_eq!(
        bench.script.borrow().actions,
        [
            Action::PciPlace,
            Action::CoreAttach,
            Action::IommuAcquire,
            Action::IommuPublish,
            Action::PciEnable,
        ]
    );

    let said = bench.release();
    assert_eq!(said.state, BindingState::Closed.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    assert_eq!(
        bench.script.borrow().actions[5..],
        [
            Action::IommuInvalidate,
            Action::PciDisable,
            Action::CoreStop,
            Action::IommuQuiesce,
            Action::IommuRelease,
            Action::PciRelease,
        ]
    );
    assert_eq!(bench.coord.core.attached(), 0);
}

#[test]
fn lost_place_ack_then_release_still_runs_all_cleanup() {
    let mut bench = setup();
    bench.script.borrow_mut().hold_pci = Some((crate::pci_api::PLACE, 5));
    let said = bench.prepare(5);
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Place));
    assert_eq!(op(&bench.coord).bar, None);

    let said = bench.release();
    assert_eq!(said.state, BindingState::Closing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Place));
    assert!(op(&bench.coord).closing);
    assert!(op(&bench.coord).exchange.is_some());

    bench.script.borrow_mut().hold_pci = None;
    bench.stages.clear();
    bench.release();
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    assert!(op(&bench.coord).bar.is_some(), "late Place ACK is reconciled");
    let actions = &bench.script.borrow().actions;
    assert!(!actions.contains(&Action::PciEnable), "no late Enable after close");
    assert!(!actions.contains(&Action::CoreAttach), "no late attach after close");
    assert_order(&bench.stages, &[
        Stage::Invalidate, Stage::Disable, Stage::Stop, Stage::Quiesce,
        Stage::Release, Stage::Detach, Stage::CloseHandoff, Stage::Unplace,
    ]);
}

#[test]
fn lost_acquire_ack_then_release_runs_iommu_cleanup_and_keeps_binding() {
    let mut bench = setup();
    bench.script.borrow_mut().hold_iommu = Some((crate::iommu_api::ACQUIRE, 5));
    let said = bench.prepare(5);
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Acquire));
    assert_eq!(op(&bench.coord).serial, 5);
    assert_eq!(op(&bench.coord).iova, 0, "range is not reconciled before the ACK");

    bench.release();
    assert_eq!(stage(&bench.coord), Some(Stage::Acquire));
    assert!(op(&bench.coord).exchange.is_some());

    bench.script.borrow_mut().hold_iommu = None;
    bench.stages.clear();
    bench.release();
    assert_eq!(op(&bench.coord).serial, 5);
    assert_eq!(op(&bench.coord).iova, 0x2_0000_0000, "late Acquire ACK reconciles the range");
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    let actions = &bench.script.borrow().actions;
    assert!(actions.contains(&Action::IommuInvalidate));
    assert!(actions.contains(&Action::IommuRelease));
    assert!(actions.contains(&Action::CoreStop));
    assert_order(&bench.stages, &[
        Stage::Invalidate, Stage::Disable, Stage::Stop, Stage::Quiesce,
        Stage::Release, Stage::Detach, Stage::CloseHandoff, Stage::Unplace,
    ]);
}

#[test]
fn attach_error_after_core_owns_map_still_stops_and_detaches() {
    let mut bench = setup();
    state::STATE.with(|state| state.borrow_mut().grant_fail = true);
    let said = bench.prepare(5);
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Attach));
    assert!(op(&bench.coord).bar.is_some());
    assert_eq!(bench.coord.core.attached(), 1);
    assert_eq!(bench.coord.core.bar_grant(5), Err(crate::recovery_api::Fail::Conflict));
    assert_eq!(state::STATE.with(|state| state.borrow().opens), 1);

    state::STATE.with(|state| state.borrow_mut().grant_fail = false);
    bench.release();
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    assert!(bench.script.borrow().actions.contains(&Action::CoreStop));
    assert_eq!(bench.coord.core.attached(), 0);
}

#[test]
fn failed_handoff_stays_handoff_and_never_re_enables() {
    let mut bench = setup();
    handoff::fail();
    let said = bench.prepare(5);
    assert_eq!(said.state, BindingState::Preparing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Handoff));
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|action| **action == Action::PciEnable).count(),
        1
    );

    handoff::reset();
    let said = bench.resume_prepare(5);
    assert_eq!(said.state, BindingState::Active.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Active));
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|action| **action == Action::PciEnable).count(),
        1
    );
}

#[test]
fn release_before_prepare_closes_and_blocks_late_prepare() {
    let mut bench = setup();
    bench.release();
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    let before = bench.script.borrow().actions.len();
    let said = bench.turn(prepare(5, EXEC, KEY));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
    assert_eq!(bench.script.borrow().actions.len(), before);
    assert!(!bench.script.borrow().actions.contains(&Action::PciPlace));
}

#[test]
fn foreign_sender_wrong_hub_and_wrong_binding_cannot_advance() {
    let mut bench = setup();
    let said = bench.coord.turn(&bench.peers, TaskId::new(2), prepare(5, EXEC, KEY));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Denied)));
    assert!(bench.script.borrow().actions.is_empty());

    let mut wrong_hub = prepare(5, EXEC, KEY);
    if let Wire::Prepare(frame) = &mut wrong_hub {
        frame.hub = TaskId::new(2);
    }
    let said = bench.turn(wrong_hub);
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Denied)));
    assert!(bench.script.borrow().actions.is_empty());

    bench.turn(prepare(5, EXEC, KEY));
    let said = bench.turn(release(6, EXEC, 2));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
    let said = bench.turn(Wire::Release(Release {
        op: RELEASE, hub: HUB, serial: 5, key: KEY,
        execution: TaskId::new(11), reason: 2, back: PieToken::mint(1),
    }));
    assert_eq!(said.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
}

#[test]
fn first_failure_is_immutable_after_a_different_failure() {
    let mut bench = setup();
    bench.script.borrow_mut().place_ok = false;
    bench.prepare(5);
    assert_eq!(
        op(&bench.coord).first_failure,
        Some(Failure { party: PARTY_PCI, op: crate::pci_api::PLACE, code: 0 })
    );
    assert_eq!(stage(&bench.coord), Some(Stage::Place));

    {
        let mut script = bench.script.borrow_mut();
        script.place_ok = true;
        script.place_status = 9;
    }
    let before = bench.script.borrow().pci.len();
    bench.advance();
    bench.advance();
    assert_eq!(bench.script.borrow().pci.len(), before + 1);
    assert_eq!(
        op(&bench.coord).first_failure,
        Some(Failure { party: PARTY_PCI, op: crate::pci_api::PLACE, code: 0 })
    );
}

#[test]
fn retry_reuses_retained_bar_and_does_not_place_again() {
    let mut bench = setup();
    state::STATE.with(|state| state.borrow_mut().grant_fail = true);
    bench.prepare(5);
    let bar = op(&bench.coord).bar;
    assert!(bar.is_some());
    assert_eq!(stage(&bench.coord), Some(Stage::Attach));
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|a| **a == Action::PciPlace).count(),
        1
    );

    state::STATE.with(|state| state.borrow_mut().grant_fail = false);
    let said = bench.resume_prepare(5);
    assert_eq!(said.state, BindingState::Active.code());
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|a| **a == Action::PciPlace).count(),
        1
    );
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|a| **a == Action::CoreAttach).count(),
        2
    );
    assert_eq!(op(&bench.coord).bar, bar);
    assert_eq!(stage(&bench.coord), Some(Stage::Active));
}

#[test]
fn failed_push_reuses_the_same_writer_without_reenabling() {
    let mut bench = setup();
    handoff::push_ok(false);
    bench.prepare(5);
    let writer = op(&bench.coord).handoff;
    assert!(writer.is_some());
    assert_eq!(stage(&bench.coord), Some(Stage::Handoff));
    bench.turn(prepare(5, EXEC, KEY));
    bench.advance();
    assert_eq!(op(&bench.coord).handoff, writer);
    assert_eq!(handoff::gives(), 1);
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|action| **action == Action::PciEnable).count(),
        1
    );
    handoff::push_ok(true);
    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Active));
    assert_eq!(handoff::gives(), 1);
}

#[test]
fn failed_writer_reclamation_retains_its_handle_and_blocks_closed_reply() {
    let mut bench = setup();
    bench.prepare(5);
    let writer = op(&bench.coord).handoff;
    handoff::release_ok(false);
    let said = bench.release();
    assert_eq!(said.state, BindingState::Closing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::CloseHandoff));
    assert_eq!(op(&bench.coord).handoff, writer);
    assert!(!bench.script.borrow().actions.contains(&Action::PciRelease));
    assert_eq!(handoff::releases(), 0);

    handoff::release_ok(true);
    bench.resume_release(5);
    assert_eq!(op(&bench.coord).handoff, None);
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    assert_eq!(handoff::releases(), 2);
}

#[test]
fn cached_bar_responsibility_survives_closed_binding_and_the_next_prepare() {
    let mut bench = setup();
    bench.prepare(5);
    let bar = op(&bench.coord).bar;
    bench.release();
    assert_eq!(op(&bench.coord).bar, bar);

    bench.script.borrow_mut().place_ok = false;
    bench.turn(prepare(6, EXEC, KEY));
    bench.advance();
    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Place));
    assert_eq!(op(&bench.coord).bar, bar);
}

#[test]
fn a_foreign_device_key_or_invalid_reason_cannot_create_a_cleanup_record() {
    let mut bench = setup();
    let mut request = release(5, EXEC, 2);
    if let Wire::Release(frame) = &mut request { frame.key = 8; }
    assert_ne!(bench.turn(request).status, wire::OK);
    assert!(bench.coord.operation.is_none());
    assert!(bench.script.borrow().actions.is_empty());

    assert_ne!(bench.turn(release(5, EXEC, 0)).status, wire::OK);
    assert!(bench.coord.operation.is_none());
    assert!(bench.script.borrow().actions.is_empty());

    bench.prepare(5);
    let count = bench.script.borrow().actions.len();
    assert_ne!(bench.turn(release(5, EXEC, 0)).status, wire::OK);
    assert_eq!(stage(&bench.coord), Some(Stage::Active));
    assert_eq!(op(&bench.coord).reason, hub_api::Close::Normal);
    assert_eq!(bench.script.borrow().actions.len(), count);
}

#[test]
fn each_release_yields_after_one_bounded_step_and_retains_the_next_stage() {
    let mut bench = setup();
    bench.prepare(5);
    let count = bench.script.borrow().actions.len();
    let said = bench.turn(release(5, EXEC, 1));
    assert_eq!(said.state, BindingState::Closing.code());
    assert_eq!(stage(&bench.coord), Some(Stage::Invalidate));
    assert_eq!(bench.script.borrow().actions.len(), count, "registration has no effect");

    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Invalidate));
    assert_eq!(bench.script.borrow().actions.len(), count, "one request begins");

    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Disable));
    assert_eq!(bench.script.borrow().actions[count..], [Action::IommuInvalidate]);

    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Disable));
    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Stop));
    assert_eq!(
        bench.script.borrow().actions[count..],
        [Action::IommuInvalidate, Action::PciDisable]
    );

    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Stop));
    assert_eq!(
        bench.script.borrow().actions[count..],
        [Action::IommuInvalidate, Action::PciDisable, Action::CoreStop]
    );

    bench.advance();
    assert_eq!(stage(&bench.coord), Some(Stage::Quiesce));
    assert_eq!(
        bench.script.borrow().actions[count..],
        [Action::IommuInvalidate, Action::PciDisable, Action::CoreStop]
    );
}

#[test]
fn explicit_supply_requires_the_exact_work_and_binding_and_is_immutable_on_retry() {
    use crate::client::{Supply, Wire as Client, SUPPLY};
    let mut bench = setup();
    bench.arrive_acquire(5);
    let endpoint = op(&bench.coord).client;
    assert!(endpoint.is_some());
    assert_eq!(op(&bench.coord).stage, Stage::Acquire);
    assert!(op(&bench.coord).source.is_none());
    assert!(bench.script.borrow().iommu.is_empty(), "no ambient gift acquisition while awaiting Supply");

    let frame = Supply { op: SUPPLY, hub: HUB, serial: 5, source: PieToken::mint(0xd1),
        access: env::Access::FETCH_STORE.bits().bits(), back: PieToken::mint(1) };
    for (from, changed) in [
        (TaskId::new(17), frame),
        (EXEC, Supply { hub: TaskId::new(17), ..frame }),
        (EXEC, Supply { serial: 6, ..frame }),
        (EXEC, Supply { source: PieToken::NONE, ..frame }),
        (EXEC, Supply { access: 0, ..frame }),
    ] {
        assert_ne!(bench.coord.client(&bench.peers, from, Client::Supply(changed)).status, wire::OK);
        assert!(op(&bench.coord).source.is_none());
    }
    let said = bench.turn(prepare(5, EXEC, KEY));
    assert_eq!(said.client, PieToken::mint(0xc2));
    assert_eq!(op(&bench.coord).client, endpoint);
    assert_eq!(
        bench.coord.client(&bench.peers, EXEC, Client::Supply(frame)).status,
        wire::OK
    );
    assert_eq!(
        bench.coord.client(&bench.peers, EXEC, Client::Supply(frame)).status,
        wire::OK
    );
    let changed = Supply { source: PieToken::mint(0xd2), ..frame };
    assert_ne!(
        bench.coord.client(&bench.peers, EXEC, Client::Supply(changed)).status,
        wire::OK
    );
    assert_eq!(op(&bench.coord).source, Some((frame.source, frame.access)));
    assert_eq!(bench.resume_prepare(5).state, BindingState::Active.code());
    assert_eq!(
        bench.coord.client(&bench.peers, EXEC, Client::Supply(frame)).status,
        wire::OK,
        "a lost Supply acknowledgement may arrive after activation"
    );
    bench.turn(release(5, EXEC, 2));
    assert_ne!(
        bench.coord.client(&bench.peers, EXEC, Client::Supply(frame)).status,
        wire::OK
    );
    assert_eq!(op(&bench.coord).source, Some((frame.source, frame.access)));
    bench.release_serial(5, 2);
    assert!(op(&bench.coord).client.is_none());
}

#[test]
fn a_closed_binding_stays_closed_without_recreating_its_client() {
    let mut bench = setup();
    assert_eq!(bench.prepare(5).state, BindingState::Active.code());
    assert_eq!(bench.release().state, BindingState::Closed.code());
    assert!(op(&bench.coord).client.is_none());
    assert!(op(&bench.coord).first_failure.is_none());
    let actions = bench.script.borrow().actions.clone();
    for _ in 0..8 { assert!(!bench.advance()); }
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    assert!(op(&bench.coord).client.is_none());
    assert!(op(&bench.coord).first_failure.is_none());
    assert_eq!(bench.script.borrow().actions, actions);
}

#[test]
fn busy_handoff_yields_with_the_same_writer_and_original_deadline() {
    let mut bench = setup();
    handoff::push_busy(true);
    bench.prepare(5);
    assert_eq!(stage(&bench.coord), Some(Stage::Handoff));
    assert!(op(&bench.coord).first_failure.is_none());
    let writer = op(&bench.coord).handoff;
    crate::chrono::set(2_999_000_000);
    assert!(!bench.advance());
    assert!(op(&bench.coord).first_failure.is_none());
    crate::chrono::set(3_000_000_000);
    assert!(!bench.advance());
    let failure = op(&bench.coord).first_failure.expect("original send budget expired");
    assert_eq!((failure.party, failure.op, failure.code), (hub_api::PARTY_RECOVERY, crate::pci_api::ENABLE, 0));
    assert_eq!(op(&bench.coord).handoff, writer);
    handoff::push_busy(false);
    assert!(bench.advance());
    assert_eq!(stage(&bench.coord), Some(Stage::Active));
    assert_eq!(op(&bench.coord).first_failure, Some(failure));
    assert_eq!(handoff::gives(), 1);
}

#[test]
fn one_pending_coordinator_does_not_block_another_with_shared_peers() {
    let mut group = group();
    group.script.borrow_mut().hold_pci = Some((crate::pci_api::PLACE, 5));
    let first = group.prepare(0, 5);
    assert_eq!(first.state, BindingState::Preparing.code());
    assert_eq!(stage(&group.coords[0]), Some(Stage::Place));
    assert!(op(&group.coords[0]).exchange.is_some());

    let second = group.prepare(1, 6);
    assert_eq!(second.state, BindingState::Active.code());
    assert_eq!(stage(&group.coords[1]), Some(Stage::Active));
    assert_eq!(op(&group.coords[1]).serial, 6);
    assert_eq!(op(&group.coords[1]).iova, 0x2_0020_0000);
    assert_eq!(stage(&group.coords[0]), Some(Stage::Place));
    assert_eq!(group.coords[0].core.attached(), 0);
    assert_eq!(group.coords[1].core.attached(), 1);

    group.script.borrow_mut().hold_pci = None;
    let first = group.prepare(0, 5);
    assert_eq!(first.state, BindingState::Active.code());
    assert_eq!(op(&group.coords[0]).iova, 0x2_0000_0000);
    assert_ne!(op(&group.coords[0]).iova, op(&group.coords[1]).iova);
    assert_ne!(op(&group.coords[0]).bar, op(&group.coords[1]).bar);
}

#[test]
fn late_enable_ack_after_release_never_handoffs_or_activates() {
    let mut bench = setup();
    bench.script.borrow_mut().hold_pci = Some((crate::pci_api::ENABLE, 5));
    bench.prepare(5);
    assert_eq!(stage(&bench.coord), Some(Stage::Enable));
    assert!(op(&bench.coord).exchange.is_some());
    assert_eq!(handoff::gives(), 0);

    bench.release();
    assert_eq!(stage(&bench.coord), Some(Stage::Enable));
    assert!(op(&bench.coord).closing);
    assert!(op(&bench.coord).exchange.is_some());

    bench.script.borrow_mut().hold_pci = None;
    bench.stages.clear();
    bench.release();
    assert_eq!(stage(&bench.coord), Some(Stage::Done));
    assert_eq!(handoff::gives(), 0, "no Handoff after close");
    assert_eq!(
        bench.script.borrow().actions.iter().filter(|action| **action == Action::PciEnable).count(),
        1,
        "the late ACK is not a new Enable"
    );
    assert_order(&bench.stages, &[
        Stage::Invalidate, Stage::Disable, Stage::Stop, Stage::Quiesce,
        Stage::Release, Stage::Detach, Stage::CloseHandoff, Stage::Unplace,
    ]);
    assert!(!bench.stages.contains(&Stage::Handoff));
    assert!(!bench.stages.contains(&Stage::Active));
}

#[test]
fn independent_floors_domains_and_sources_survive_neighbour_progress() {
    let mut group = group();
    group.arrive_acquire(0, 5, 0xd1);
    group.arrive_acquire(1, 6, 0xd2);
    assert_eq!(
        op(&group.coords[0]).source,
        Some((PieToken::mint(0xd1), env::Access::FETCH_STORE.bits().bits()))
    );
    assert_eq!(
        op(&group.coords[1]).source,
        Some((PieToken::mint(0xd2), env::Access::FETCH_STORE.bits().bits()))
    );
    assert_eq!(group.resume_prepare(0, 5).state, BindingState::Active.code());
    assert_eq!(group.resume_prepare(1, 6).state, BindingState::Active.code());
    assert_ne!(op(&group.coords[0]).iova, op(&group.coords[1]).iova);
    assert_ne!(group.coords[0].domain, group.coords[1].domain);
    assert_eq!(
        group.script.borrow().iommu.iter().filter_map(|wire| match wire {
            crate::iommu_api::Wire::Acquire(frame) => Some((frame.key, frame.source)),
            _ => None,
        }).collect::<Vec<_>>(),
        [(23, PieToken::mint(0xd1)), (24, PieToken::mint(0xd2))]
    );

    group.release(0, 5);
    assert_eq!(stage(&group.coords[0]), Some(Stage::Done));
    assert_eq!(group.coords[0].last_serial, 5);
    assert_eq!(stage(&group.coords[1]), Some(Stage::Active));
    assert_eq!(group.coords[1].last_serial, 0);
    assert_eq!(group.coords[0].core.attached(), 0);
    assert_eq!(group.coords[1].core.attached(), 1);
    assert_eq!(op(&group.coords[1]).serial, 6);
    assert_eq!(op(&group.coords[1]).iova, 0x2_0020_0000);

    let late = group.turn(0, prepare(5, EXEC, KEY));
    assert_eq!(late.status, hub_api::fail_to_code(Some(hub_api::Fail::Conflict)));
    assert_eq!(group.coords[1].core.attached(), 1, "the neighbour keeps its binding");
}
