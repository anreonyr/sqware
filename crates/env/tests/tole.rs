use env::{
    Bit, EnvCall, HoleCondition, PieToken, TaskId, TeamId, ToleAwaitReply, ToleCall, ToleEventFail,
    ToleEventSource, ToleFail, ToleResult, UnitTarget, Wait,
};

fn sources() -> [ToleEventSource; 9] {
    let token = PieToken::mint(7);
    [
        ToleEventSource::Hole {
            token,
            condition: HoleCondition::Pull,
        },
        ToleEventSource::Hole {
            token,
            condition: HoleCondition::Push,
        },
        ToleEventSource::Hole {
            token,
            condition: HoleCondition::Empty,
        },
        ToleEventSource::Hole {
            token,
            condition: HoleCondition::Signal,
        },
        ToleEventSource::Pole {
            token,
            bit: Bit::of(usize::BITS as usize - 1).unwrap(),
        },
        ToleEventSource::Nole { token },
        ToleEventSource::Join {
            target: UnitTarget::Task(TaskId::new(3)),
        },
        ToleEventSource::Join {
            target: UnitTarget::Team(TeamId::new(4)),
        },
        ToleEventSource::Inspect {
            task: TaskId::new(3),
            token,
        },
    ]
}

#[test]
fn tole_calls_round_trip_in_class_twelve() {
    let token = PieToken::mint(7);
    let calls = [
        ToleCall::Unseal { shared: true },
        ToleCall::Seal { token },
        ToleCall::Attach {
            tole: token,
            source: sources()[0],
        },
        ToleCall::Detach {
            tole: token,
            source: sources()[0],
        },
        ToleCall::Await {
            tole: token,
            millis: Wait::Forever,
        },
    ];
    for (slot, call) in calls.into_iter().enumerate() {
        assert_eq!(call.slot(), (12usize << 32) | slot);
        assert_eq!(
            EnvCall::from_wire(call.slot(), &call.pack()),
            Ok(EnvCall::Tole(call))
        );
    }
    assert_eq!(calls[0].pack(), [1, 0, 0, 0, 0, 0]);
    assert_eq!(calls[4].pack(), [7, usize::MAX, 0, 0, 0, 0]);
    assert!(ToleCall::from_wire(calls[0].slot(), &[2, 0, 0, 0, 0, 0]).is_err());
    assert!(EnvCall::from_wire((12usize << 32) | 5, &[0; 6]).is_err());
}

#[test]
fn sources_fit_three_words_and_attach_fits_four() {
    let expected = [
        [1, 7, 0],
        [1, 7, 1],
        [1, 7, 2],
        [1, 7, 3],
        [5, 7, usize::BITS as usize - 1],
        [6, 7, 0],
        [2, 3, 0],
        [3, 4, 0],
        [4, 3, 7],
    ];
    for (source, expected) in sources().into_iter().zip(expected) {
        let words = source.words();
        assert_eq!(words, expected);
        assert_eq!(ToleEventSource::of(words), Some(source));
        let call = ToleCall::Attach {
            tole: PieToken::mint(11),
            source,
        };
        assert_eq!(call.pack(), [11, words[0], words[1], words[2], 0, 0]);
        assert_eq!(ToleCall::from_wire(call.slot(), &call.pack()), Ok(call));
        let call = ToleCall::Detach {
            tole: PieToken::mint(11),
            source,
        };
        assert_eq!(ToleCall::from_wire(call.slot(), &call.pack()), Ok(call));
    }
    for words in [
        [0, 7, 0],
        [1, 0, 0],
        [1, 7, 4],
        [2, 3, 1],
        [3, 4, 1],
        [4, 3, 0],
        [5, 7, usize::BITS as usize],
        [5, 0, 0],
        [6, 0, 0],
        [6, 7, 1],
        [7, 7, 0],
    ] {
        assert!(ToleEventSource::of(words).is_none(), "{words:?}");
        let call = ToleCall::Attach {
            tole: PieToken::mint(11),
            source: sources()[0],
        };
        assert!(
            ToleCall::from_wire(call.slot(), &[11, words[0], words[1], words[2], 0, 0]).is_err()
        );
    }
}

#[test]
fn await_triples_preserve_source_identity_and_typed_event_failures() {
    assert_eq!(ToleAwaitReply::Pending.words(), [0; 3]);
    assert_eq!(ToleAwaitReply::of([0; 3]), Some(ToleAwaitReply::Pending));
    let failures = [
        None,
        Some(ToleEventFail::Denied),
        Some(ToleEventFail::Dead),
        Some(ToleEventFail::Busy),
        Some(ToleEventFail::OoM),
        Some(ToleEventFail::HandedOver),
        Some(ToleEventFail::Gone),
    ];
    for source in sources() {
        for fail in failures {
            let reply = ToleAwaitReply::Source { source, fail };
            let words = reply.words();
            assert_eq!(words[0] & 255, source.words()[0]);
            assert_eq!(words[0] >> 8, fail.map_or(0, |e| -e.code() as usize));
            assert_eq!(&words[1..], &source.words()[1..]);
            assert_eq!(ToleAwaitReply::of(words), Some(reply));
        }
    }
    assert!(ToleAwaitReply::of([1 | (7 << 8), 7, 0]).is_none());
    assert!(ToleAwaitReply::of([0, 7, 0]).is_none());
    let _: fn(PieToken, Wait) -> ToleResult<ToleAwaitReply> = env::tole::await_;
    assert_eq!(ToleFail::HandedOver.code(), -5);
}
