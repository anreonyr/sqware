use env::{EnvCall, NoleCall, PieToken, Wait};

#[test]
fn nole_domain_has_five_compact_bit_free_calls() {
    let token = PieToken::mint(7);
    for (slot, call, words) in [
        (0, NoleCall::Unseal, [0; 6]),
        (1, NoleCall::Seal { token }, [7, 0, 0, 0, 0, 0]),
        (
            2,
            NoleCall::Wait {
                token,
                millis: Wait::POLL,
            },
            [7, 0, 0, 0, 0, 0],
        ),
        (3, NoleCall::Ring { token }, [7, 0, 0, 0, 0, 0]),
        (4, NoleCall::Hush { token }, [7, 0, 0, 0, 0, 0]),
    ] {
        assert_eq!(call.slot(), (11usize << 32) | slot);
        assert_eq!(call.pack(), words);
        assert_eq!(
            EnvCall::from_wire(call.slot(), &words),
            Ok(EnvCall::Nole(call))
        );
    }
    assert!(EnvCall::from_wire((11usize << 32) | 5, &[0; 6]).is_err());
    for slot in 0..64 {
        assert!(EnvCall::from_wire((9usize << 32) | slot, &[0; 6]).is_err());
    }
}
