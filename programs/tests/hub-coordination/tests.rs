use env::{PieToken, TaskId};
use env::wire::Span as _;
use hub_api::coordination::{self, Prepare, Release, Wire};
use hub_api::{BindingState, DmaSaid, Failure, PARTY_HUB};
use wire::{Contract, Message};

fn token(raw: u64) -> PieToken {
    PieToken::mint(raw as usize)
}

fn prepare(hub: usize, serial: u64, back: PieToken) -> Prepare {
    Prepare {
        op: coordination::PREPARE,
        hub: TaskId::new(hub),
        serial,
        key: 0xAABB_CCDD_1122_3344,
        execution: TaskId::new(11),
        back,
    }
}

fn release(hub: usize, serial: u64, back: PieToken) -> Release {
    Release {
        op: coordination::RELEASE,
        hub: TaskId::new(hub),
        serial,
        key: 0xAABB_CCDD_1122_3344,
        execution: TaskId::new(11),
        reason: 2,
        back,
    }
}

#[test]
fn prepare_roundtrips_with_qualified_binding_identity() {
    let back = token(3);
    let wire = Wire::Prepare(prepare(7, 9, back));
    let mut bytes = <Wire as Message>::EMPTY;
    let len = wire.store(&mut bytes).unwrap();
    assert_eq!(len, Prepare::LEN);
    assert_eq!(Wire::fetch(&bytes[..len]), Some(wire));

    let Wire::Prepare(decoded) = Wire::fetch(&bytes[..len]).unwrap() else {
        panic!("expected prepare");
    };
    assert_eq!(decoded.binding().hub, TaskId::new(7));
    assert_eq!(decoded.binding().serial, 9);
    assert_eq!(decoded.key, 0xAABB_CCDD_1122_3344);
    assert_eq!(decoded.execution, TaskId::new(11));
    assert_eq!(decoded.back, back);
}

#[test]
fn release_roundtrips_and_preserves_binding_identity() {
    let back = token(4);
    let wire = Wire::Release(release(8, 10, back));
    let mut bytes = <Wire as Message>::EMPTY;
    let len = wire.store(&mut bytes).unwrap();
    assert_eq!(len, Release::LEN);
    assert_eq!(Wire::fetch(&bytes[..len]), Some(wire));

    let Wire::Release(decoded) = Wire::fetch(&bytes[..len]).unwrap() else {
        panic!("expected release");
    };
    assert_eq!(decoded.binding().hub, TaskId::new(8));
    assert_eq!(decoded.binding().serial, 10);
    assert_eq!(decoded.execution, TaskId::new(11));
    assert_eq!(decoded.reason, 2);
    assert_eq!(decoded.back, back);
}

#[test]
fn prepare_truncated_and_trailing_frames_are_rejected() {
    let wire = Wire::Prepare(prepare(7, 9, token(3)));
    let mut bytes = <Wire as Message>::EMPTY;
    let len = wire.store(&mut bytes).unwrap();
    for end in 0..len {
        assert!(Wire::fetch(&bytes[..end]).is_none());
    }
    let mut extra = [0u8; Wire::LEN + 1];
    extra[..len].copy_from_slice(&bytes[..len]);
    assert!(Wire::fetch(&extra[..len + 1]).is_none());
}

#[test]
fn release_truncated_and_trailing_frames_are_rejected() {
    let wire = Wire::Release(release(8, 10, token(4)));
    let mut bytes = <Wire as Message>::EMPTY;
    let len = wire.store(&mut bytes).unwrap();
    for end in 0..len {
        assert!(Wire::fetch(&bytes[..end]).is_none());
    }
    let mut extra = [0u8; Release::LEN + 1];
    extra[..len].copy_from_slice(&bytes[..len]);
    assert!(Wire::fetch(&extra[..len + 1]).is_none());
}

#[test]
fn wrong_op_is_rejected_in_header_and_body() {
    let wire = Wire::Prepare(prepare(7, 9, token(3)));
    let mut bytes = <Wire as Message>::EMPTY;
    let len = wire.store(&mut bytes).unwrap();
    let mut wrong_header = bytes;
    wrong_header[0] = coordination::RELEASE;
    assert!(Wire::fetch(&wrong_header[..len]).is_none());

    let body = Prepare {
        op: coordination::RELEASE,
        ..prepare(7, 9, token(3))
    };
    let mut out = <Wire as Message>::EMPTY;
    assert!(Wire::Prepare(body).store(&mut out).is_none());
}

#[test]
fn same_serial_is_qualified_by_hub() {
    let left = prepare(1, 5, token(6));
    let right = prepare(2, 5, token(7));
    assert_ne!(left.binding(), right.binding());
    assert_eq!(left.binding().serial, right.binding().serial);

    let mut bytes = <Wire as Message>::EMPTY;
    let len = Wire::Prepare(left).store(&mut bytes).unwrap();
    let Wire::Prepare(decoded) = Wire::fetch(&bytes[..len]).unwrap() else {
        panic!("expected prepare");
    };
    assert_eq!(decoded.binding(), left.binding());
    assert_ne!(decoded.binding(), right.binding());
}

#[test]
fn existing_dma_said_remains_the_response_shape() {
    let failure = Failure {
        party: PARTY_HUB,
        op: coordination::PREPARE,
        code: 7,
    };
    let said = DmaSaid::progress(9, BindingState::Active, Some(failure));
    let mut bytes = <<coordination::Call as Contract>::Response as Message>::EMPTY;
    let len = said.store(&mut bytes).unwrap();
    let decoded = <<coordination::Call as Contract>::Response as Message>::fetch(&bytes[..len]).unwrap();
    assert_eq!(decoded.serial, 9);
    assert_eq!(decoded.state, BindingState::Active.code());
    assert_eq!(decoded.failure(), Some(failure));
}

#[test]
fn zero_hub_serial_or_execution_is_not_a_well_formed_request() {
    for (hub, serial, execution) in [(0, 9, 11), (7, 0, 11), (7, 9, 0)] {
        let prepare = Prepare {
            execution: TaskId::new(execution),
            ..prepare(hub, serial, token(3))
        };
        let release = Release {
            execution: TaskId::new(execution),
            ..release(hub, serial, token(4))
        };
        let mut bytes = <Wire as Message>::EMPTY;
        assert!(Wire::Prepare(prepare).store(&mut bytes).is_none());
        let n = prepare.store_at(&mut bytes, 0).unwrap();
        assert!(Wire::fetch(&bytes[..n]).is_none());
        assert!(Wire::Release(release).store(&mut bytes).is_none());
        let n = release.store_at(&mut bytes, 0).unwrap();
        assert!(Wire::fetch(&bytes[..n]).is_none());
    }
}

#[test]
fn zero_device_key_remains_an_opaque_valid_coordinate() {
    let request = Prepare {
        key: 0,
        ..prepare(7, 9, token(3))
    };
    let mut bytes = <Wire as Message>::EMPTY;
    let len = Wire::Prepare(request).store(&mut bytes).unwrap();
    assert_eq!(Wire::fetch(&bytes[..len]), Some(Wire::Prepare(request)));
}
