use env::{
    Bit, EnvCall, HoleCondition, PieToken, PoleCall, PoleCondition, TaskId, ToleAwaitReply,
    ToleCall, ToleEventFail, ToleEventSource, UnitTarget, Wait,
};
fn token(n: u64) -> PieToken {
    PieToken::from_bytes(&n.to_le_bytes()).unwrap()
}
#[test]
fn independent_notifications_reject_invalid_indices_and_roundtrip() {
    for index in 0..usize::BITS as usize {
        let bit = Bit::wrap(index);
        let wait = PoleCall::Wait {
            token: token(1),
            condition: PoleCondition::Signal(bit),
            millis: Wait::POLL,
        };
        assert_eq!(PoleCall::from_wire(wait.slot(), &wait.pack()), Ok(wait));
        for call in [
            PoleCall::Ring {
                token: token(1),
                bit,
            },
            PoleCall::Hush {
                token: token(1),
                bit,
            },
        ] {
            assert_eq!(
                PoleCall::from_wire(call.slot(), &call.pack()).unwrap(),
                call
            );
            let mut words = call.pack();
            words[1] = usize::BITS as usize;
            assert!(PoleCall::from_wire(call.slot(), &words).is_err());
        }
    }
    assert!(Bit::of(usize::BITS as usize).is_none());
    let call = PoleCall::Wait {
        token: token(1),
        condition: PoleCondition::Signal(Bit::FIRST),
        millis: Wait::POLL,
    };
    let mut words = call.pack();
    words[1] = usize::MAX;
    assert!(PoleCall::from_wire(call.slot(), &words).is_err());
}
#[test]
fn source_identity_roundtrip_preserves_resource_join_and_exact_reference() {
    for source in [
        ToleEventSource::Hole {
            token: token(1),
            condition: HoleCondition::Signal,
        },
        ToleEventSource::Pole {
            token: token(1),
            bit: Bit::of(usize::BITS as usize - 1).unwrap(),
        },
        ToleEventSource::Nole { token: token(1) },
        ToleEventSource::Join {
            target: UnitTarget::Task(TaskId::new(2)),
        },
        ToleEventSource::Join {
            target: UnitTarget::Team(env::TeamId::new(3)),
        },
        ToleEventSource::Inspect {
            task: TaskId::new(4),
            token: token(5),
        },
    ] {
        for call in [
            ToleCall::Attach {
                tole: token(6),
                source,
            },
            ToleCall::Detach {
                tole: token(6),
                source,
            },
        ] {
            assert_eq!(
                ToleCall::from_wire(call.slot(), &call.pack()).unwrap(),
                call
            );
        }
        for fail in [
            None,
            Some(ToleEventFail::Denied),
            Some(ToleEventFail::Dead),
            Some(ToleEventFail::Busy),
            Some(ToleEventFail::OoM),
            Some(ToleEventFail::HandedOver),
            Some(ToleEventFail::Gone),
        ] {
            let reply = ToleAwaitReply::Source { source, fail };
            assert_eq!(ToleAwaitReply::of(reply.words()), Some(reply));
        }
    }
    assert_eq!(ToleAwaitReply::of([0; 3]), Some(ToleAwaitReply::Pending));
    for invalid in [
        [0, 1, 0],
        [7, 1, 0],
        [1, 0, 0],
        [2, 1, 1],
        [3, 1, 1],
        [4, 1, 0],
        [0x701, 1, 0],
        [1, 1, usize::MAX],
    ] {
        assert!(ToleAwaitReply::of(invalid).is_none());
        assert!(ToleEventSource::of(invalid).is_none());
    }
}
#[test]
fn retired_mail_and_tole_slots_are_rejected() {
    for slot in 8..=40 {
        assert!(EnvCall::from_wire((5usize << 32) | slot, &[0; 6]).is_err());
    }
    for slot in 0..64 {
        assert!(EnvCall::from_wire((9usize << 32) | slot, &[0; 6]).is_err());
    }
    let call = ToleCall::Await {
        tole: token(1),
        millis: Wait::POLL,
    };
    assert_eq!(call.slot(), (12usize << 32) | 4);
    assert_eq!(
        EnvCall::from_wire(call.slot(), &call.pack()),
        Ok(EnvCall::Tole(call))
    );
}
