use env::{MemoryCall, PieCall, PieToken, TeamId, UnsealArgs, VirtAddr};

#[test]
fn pole_creation_encodes_an_existing_mapping() {
    for shared in [false, true] {
        let call = PieCall::Unseal {
            args: UnsealArgs::Pole {
                addr: VirtAddr::new(0x4100_0000),
                size: 8192,
                shared,
            },
        };
        assert_eq!(call.pack(), [1, 0x4100_0000, 8192, shared as usize, 0, 0]);
        assert_eq!(PieCall::from_wire(call.slot(), &call.pack()).unwrap(), call);
        let mut words = call.pack();
        words[3] = 2;
        assert!(PieCall::from_wire(call.slot(), &words).is_err());
        words = call.pack();
        words[4] = 1;
        assert!(PieCall::from_wire(call.slot(), &words).is_err());
    }
}

#[test]
fn mmap_encodes_pole_offset_and_target_domain() {
    let call = MemoryCall::Mmap {
        team: TeamId::new(7),
        at: VirtAddr::new(0x5000_0000),
        size: 8192,
        pole: PieToken::from_bytes(&13u64.to_le_bytes()).unwrap(),
        offset: 4096,
        flags: 10,
    };
    assert_eq!(call.pack(), [7, 0x5000_0000, 8192, 13, 4096, 10]);
    assert_eq!(
        MemoryCall::from_wire(call.slot(), &call.pack()).unwrap(),
        call
    );
}
