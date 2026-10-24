//! Exact provider frames: fixed offsets come from the one field table, lengths are exact.

use crate::{
    PieToken, TaskId,
    api::{
        ACQUIRE, Acquire, ESTABLISH, Establish, INVALIDATE, PUBLISH, QUIESCE, RELEASE, Said, Step,
        Wire,
    },
};
use wire::Message;

const ENGINE: TaskId = TaskId::new(5);
const HUB: TaskId = TaskId::new(3);
const WORK: TaskId = TaskId::new(9);

fn store(wire: &Wire) -> ([u8; 64], usize) {
    let mut bytes = [0u8; 64];
    let end = wire.store(&mut bytes).expect("encodes");
    (bytes, end)
}

#[test]
fn frame_sums_and_receive_buffers_match_the_contract() {
    assert_eq!(Establish::LEN, 13);
    assert_eq!(Acquire::LEN, 61);
    assert_eq!(Step::LEN, 42);
    assert_eq!(Said::LEN, 38);
    assert_eq!(Wire::LEN, 64);
}

#[test]
fn every_frame_round_trips_at_its_exact_length_and_rejects_truncation_or_extra_bytes() {
    let cases = [
        (
            Wire::Establish(Establish {
                op: ESTABLISH,
                requester: 9,
                back: PieToken::mint(1),
            }),
            13,
        ),
        (
            Wire::Acquire(Acquire {
                op: ACQUIRE,
                engine: ENGINE,
                key: 7,
                hub: HUB,
                serial: 11,
                execution: WORK,
                source: PieToken::mint(42),
                access: 6,
                back: PieToken::mint(2),
            }),
            61,
        ),
        (
            Wire::Publish(Step {
                op: PUBLISH,
                engine: ENGINE,
                key: 7,
                hub: HUB,
                serial: 11,
                reason: 0,
                back: PieToken::mint(3),
            }),
            42,
        ),
        (
            Wire::Invalidate(Step {
                op: INVALIDATE,
                engine: ENGINE,
                key: 7,
                hub: HUB,
                serial: 11,
                reason: 0,
                back: PieToken::mint(4),
            }),
            42,
        ),
        (
            Wire::Quiesce(Step {
                op: QUIESCE,
                engine: ENGINE,
                key: 7,
                hub: HUB,
                serial: 11,
                reason: 0,
                back: PieToken::mint(5),
            }),
            42,
        ),
        (
            Wire::Release(Step {
                op: RELEASE,
                engine: ENGINE,
                key: 7,
                hub: HUB,
                serial: 11,
                reason: 2,
                back: PieToken::mint(6),
            }),
            42,
        ),
    ];
    for (wire, length) in cases {
        let (bytes, end) = store(&wire);
        assert_eq!(end, length);
        assert_eq!(Wire::take(&bytes[..end]), Some(wire));
        assert_eq!(Wire::take(&bytes[..end - 1]), None, "truncation");
        assert_eq!(Wire::take(&bytes[..end + 1]), None, "extra byte");
    }
}

#[test]
fn wrong_or_unknown_action_codes_are_rejected_before_any_effect() {
    let (mut bytes, end) = store(&Wire::Establish(Establish {
        op: ESTABLISH,
        requester: 9,
        back: PieToken::mint(1),
    }));
    for op in [0, ACQUIRE, PUBLISH, INVALIDATE, QUIESCE, RELEASE, 7, 255] {
        bytes[0] = op;
        assert_eq!(Wire::take(&bytes[..end]), None, "opcode {op}");
    }
    bytes[0] = ESTABLISH;
    assert_eq!(Wire::take(&bytes[..end]), Some(Wire::Establish(Establish {
        op: ESTABLISH,
        requester: 9,
        back: PieToken::mint(1),
    })));
}

#[test]
fn said_is_38_bytes_with_the_domain_and_private_allocation_echoed() {
    let said = Said {
        status: 1,
        pending: 1,
        held: 5,
        engine: ENGINE,
        key: 7,
        iova: 8 << 30,
        bytes: 3 * crate::PAGE_SIZE as u64,
    };
    let mut bytes = [0u8; 48];
    let end = said.store(&mut bytes).unwrap();
    assert_eq!(end, 38);
    assert_eq!(<Said as Message>::fetch(&bytes[..38]), Some(said));
    assert_eq!(<Said as Message>::fetch(&bytes[..37]), None);
    assert_eq!(<Said as Message>::fetch(&bytes[..48]), None);
}
