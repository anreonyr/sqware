use env::{AwaitReply, Bit, EnvCall, MailCall, MailCondition, MailFail, PieToken, Source, TaskId, UnitTarget, Wait, Wire};
fn token(n: u64) -> PieToken { PieToken::from_bytes(&n.to_le_bytes()).unwrap() }
#[test]
fn independent_notifications_reject_invalid_indices_and_roundtrip() {
    for index in 0..usize::BITS as usize {
        let bit = Bit::of(index).unwrap(); let condition = MailCondition::Signal(bit);
        assert_eq!(MailCondition::of(condition.wire()), Some(condition));
        for call in [MailCall::Ring { token: token(1), bit }, MailCall::Hush { token: token(1), bit }] {
            assert_eq!(MailCall::from_wire(call.slot(), &call.pack()).unwrap(), call);
            let mut words = call.pack(); words[1] = usize::BITS as usize;
            assert!(MailCall::from_wire(call.slot(), &words).is_err());
        }
    }
    assert!(Bit::of(usize::BITS as usize).is_none());
    assert!(MailCondition::of(usize::MAX).is_none());
}
#[test]
fn source_identity_roundtrip_preserves_mail_join_and_exact_reference() {
    for source in [
        Source::Mail { pie: token(1), condition: MailCondition::Signal(Bit::of(63).unwrap()) },
        Source::Join { target: UnitTarget::Task(TaskId::new(2)) },
        Source::Join { target: UnitTarget::Team(env::TeamId::new(3)) },
        Source::Inspect { task: TaskId::new(4), token: token(5) },
    ] {
        for call in [MailCall::Attach { tole: token(6), source }, MailCall::Detach { tole: token(6), source }] {
            assert_eq!(MailCall::from_wire(call.slot(), &call.pack()).unwrap(), call);
        }
        for fail in [None, Some(MailFail::Denied), Some(MailFail::Dead), Some(MailFail::Busy), Some(MailFail::OoM), Some(MailFail::HandedOver), Some(MailFail::Gone)] {
            let reply = AwaitReply::Source { source, fail };
            assert_eq!(AwaitReply::of(reply.words()), Some(reply));
        }
    }
    assert_eq!(AwaitReply::of([0; 3]), Some(AwaitReply::Pending));
    for invalid in [[0, 1, 0], [5, 1, 0], [1, 0, 0], [2, 1, 1], [3, 1, 1], [4, 1, 0], [0x701, 1, 0], [1, 1, usize::MAX]] {
        assert!(AwaitReply::of(invalid).is_none()); assert!(Source::of(invalid).is_none());
    }
}
#[test]
fn retired_mail_and_tole_slots_are_rejected() {
    for class in [5, 9] { for slot in 0..32 { assert!(EnvCall::from_wire((class << 32) | slot, &[0; 6]).is_err()); } }
    let call = MailCall::Await { tole: token(1), millis: Wait::POLL };
    assert_eq!(call.slot(), (5usize << 32) | 40);
    assert_eq!(EnvCall::from_wire(call.slot(), &call.pack()), Ok(EnvCall::Mail(call)));
}
