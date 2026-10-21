use env::{
    Bit, EnvCall, FlatReference, HoleCall, HoleCondition, HoleLimits, Mark, PieToken, PoleCall,
    PoleCondition, Wait,
};

#[test]
fn hole_creation_has_four_words_and_rejects_invalid_limits() {
    let call = HoleCall::Unseal {
        mark: Mark::new(17),
        limits: HoleLimits {
            max_len: 8,
            max_messages: 3,
            max_bytes: 24,
        },
    };
    assert_eq!(call.slot(), 5usize << 32);
    assert_eq!(call.pack(), [17, 8, 3, 24, 0, 0]);
    assert_eq!(
        EnvCall::from_wire(call.slot(), &call.pack()),
        Ok(EnvCall::Hole(call))
    );
    for words in [
        [17, 0, 3, 24, 0, 0],
        [17, 8, 0, 24, 0, 0],
        [17, 8, 3, 7, 0, 0],
    ] {
        assert!(HoleCall::from_wire(call.slot(), &words).is_err());
    }
}

#[test]
fn hole_condition_has_no_index_and_notification_calls_have_one_word() {
    let token = PieToken::mint(7);
    let call = HoleCall::Wait {
        token,
        condition: HoleCondition::Signal,
        millis: Wait::AtMost(29),
    };
    assert_eq!(call.pack(), [7, 3, 29, 0, 0, 0]);
    assert!(HoleCall::from_wire(call.slot(), &[7, 4, 29, 0, 0, 0]).is_err());
    for call in [HoleCall::Ring { token }, HoleCall::Hush { token }] {
        assert_eq!(call.pack(), [7, 0, 0, 0, 0, 0]);
        assert_eq!(HoleCall::from_wire(call.slot(), &call.pack()), Ok(call));
    }
}

#[test]
fn pole_creation_has_two_words_and_checks_boolean() {
    let call = PoleCall::Unseal {
        reference: FlatReference::from_bytes(&9u64.to_le_bytes()).unwrap(),
        shared: true,
    };
    assert_eq!(call.slot(), 10usize << 32);
    assert_eq!(call.pack(), [9, 1, 0, 0, 0, 0]);
    assert_eq!(
        EnvCall::from_wire(call.slot(), &call.pack()),
        Ok(EnvCall::Pole(call))
    );
    assert!(PoleCall::from_wire(call.slot(), &[9, 2, 0, 0, 0, 0]).is_err());
}

#[test]
fn pole_wait_preserves_indexed_signal_encoding() {
    for index in 0..usize::BITS as usize {
        let call = PoleCall::Wait {
            token: PieToken::mint(7),
            condition: PoleCondition::Signal(Bit::of(index).unwrap()),
            millis: Wait::Forever,
        };
        assert_eq!(call.pack(), [7, index + 3, usize::MAX, 0, 0, 0]);
        assert_eq!(PoleCall::from_wire(call.slot(), &call.pack()), Ok(call));
        assert!(PoleCall::from_wire(call.slot(), &[7, usize::MAX, 0, 0, 0, 0]).is_err());
    }
}
