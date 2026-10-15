use env::{
    AcquireSource, Allocation, Decode, FlatReference, FromPair, MemoryCall, PieCall, PieToken,
    Placement, PlacementAt, ReferenceRange, TeamId, UnsealArgs, ViewControl, ViewId, ViewSelection,
    VirtAddr, Wire,
};

#[test]
fn pole_creation_takes_a_complete_source_reference() {
    for shared in [false, true] {
        let reference = reference_of(0x4100);
        let call = PieCall::Unseal {
            args: UnsealArgs::Pole { reference, shared },
        };
        assert_eq!(call.pack(), [1, 0x4100, 0, 0, shared as usize, 0]);
        assert_eq!(PieCall::from_wire(call.slot(), &call.pack()).unwrap(), call);
        let mut words = call.pack();
        words[3] = 2;
        assert!(PieCall::from_wire(call.slot(), &words).is_err());
        words = call.pack();
        words[4] = 2;
        assert!(PieCall::from_wire(call.slot(), &words).is_err());
    }
}

#[test]
fn source_acquisition_keeps_the_pole_window_and_attach_keeps_the_target_domain() {
    let source = MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: PieToken::from_bytes(&13u64.to_le_bytes()).unwrap(),
            offset: 1,
            count: 2,
        },
        retention: PieToken::NONE,
    };
    assert_eq!(source.pack(), [2, 13, 1, 2, 0, 0]);
    assert_eq!(
        MemoryCall::from_wire(source.slot(), &source.pack()).unwrap(),
        source
    );
    let call = MemoryCall::Attach {
        team: TeamId::new(7),
        range: VirtAddr::new(0x4000),
        placement: VirtAddr::new(0x5000),
        access: 10,
        control: ViewControl::SpaceMembers,
    };
    assert_eq!(call.pack(), [0x4000, 7, 0x5000, 10, 1, 0]);
    assert_eq!(
        MemoryCall::from_wire(call.slot(), &call.pack()).unwrap(),
        call
    );
}

/// 由线上那格取一枚句柄（外部测试只有解码这一扇门）。
fn reference_of(raw: usize) -> FlatReference {
    match MemoryCall::from_wire((2usize << 32) | 6, &[raw, 0, 0, 0, 0, 0]).unwrap() {
        MemoryCall::Release { reference } => reference,
        other => panic!("unexpected release: {other:?}"),
    }
}

fn decode(slot: usize, regs: [usize; 6]) -> MemoryCall {
    MemoryCall::from_wire((2usize << 32) | slot, &regs).unwrap()
}

#[test]
fn memory_slots_reject_the_retired_five_and_keep_explicit_lifecycle_slots() {
    for slot in 0..5usize {
        for regs in [
            [0; 6],
            [usize::MAX; 6],
            [7, 0x5000_0000, 8192, 13, 4096, 10],
        ] {
            assert_eq!(
                MemoryCall::from_wire((2usize << 32) | slot, &regs),
                Err(Decode::BadSlot)
            );
        }
    }
    for (slot, regs) in [
        (5, [0, 4096, 4096, 0, 0, 0]),
        (6, [7, 0, 0, 0, 0, 0]),
        (7, [0x4000, 7, 0x5000, 10, 1, 0]),
        (8, [3, 9, 0, 0, 0, 0]),
        (9, [3, 9, 1, 1, 2, 10]),
        (11, [99, 0x6000, 0x7000, 0, 0, 0]),
    ] {
        assert_eq!(decode(slot, regs).slot(), (2usize << 32) | slot);
    }
    // Transfer stays reserved until its safety and authority paths are implemented.
    assert_eq!(
        MemoryCall::from_wire((2usize << 32) | 10, &[0; 6]),
        Err(Decode::BadSlot)
    );
    assert_eq!(
        MemoryCall::from_wire((2usize << 32) | 12, &[0; 6]),
        Err(Decode::BadSlot)
    );
}

#[test]
fn acquire_carries_the_source_and_the_optional_reservation() {
    // Allocation(4096, 4096) + 普通取得（retention=NONE）。
    let regs = [0, 4096, 4096, 0, 0, 0];
    let call = decode(5, regs);
    assert_eq!(
        call,
        MemoryCall::Acquire {
            source: AcquireSource::Allocation(Allocation::new(4096, 4096).expect("allocation")),
            retention: PieToken::NONE,
        }
    );
    assert_eq!(call.pack(), regs);

    // Reference(range) + 独立保留授权。
    let regs = [1, 7, 1, 2, 0, 99];
    let call = decode(5, regs);
    match call {
        MemoryCall::Acquire {
            source: AcquireSource::Reference(range),
            retention,
        } => {
            assert_eq!((range.offset, range.count), (1, 2));
            assert_eq!(range.reference.get(), 7);
            assert_eq!(retention.get(), 99);
        }
        other => panic!("unexpected acquire: {other:?}"),
    }
    assert_eq!(call.pack(), regs);

    // Pole 的 Pie 与范围。
    let regs = [2, 4, 1, 3, 0, 0];
    assert_eq!(decode(5, regs).pack(), regs);

    // View 来源：域 + 句柄 + 页窗口。
    let regs = [3, 4, 11, 1, 3, 0];
    assert_eq!(decode(5, regs).pack(), regs);

    // 零页数 / 溢出 / 越界标签 / 非 2 的幂对齐都非法。
    assert!(MemoryCall::from_wire((2usize << 32) | 5, &[1, 7, 0, 0, 0, 0]).is_err());
    assert!(MemoryCall::from_wire((2usize << 32) | 5, &[1, 7, usize::MAX, 2, 0, 0]).is_err());
    assert!(MemoryCall::from_wire((2usize << 32) | 5, &[9, 0, 0, 0, 0, 0]).is_err());
    assert!(MemoryCall::from_wire((2usize << 32) | 5, &[0, 0, 4096, 0, 0, 0]).is_err());
    assert!(MemoryCall::from_wire((2usize << 32) | 5, &[0, 4096, 3000, 0, 0, 0]).is_err());
    // 第五格（来源载荷的末位）只许为零的那几支：多一位即非法。
    assert!(MemoryCall::from_wire((2usize << 32) | 5, &[1, 7, 1, 2, 1, 0]).is_err());
}

#[test]
fn traverse_encodes_authority_window_and_output_without_an_implicit_length() {
    let call = MemoryCall::Traverse {
        retention: env::PieToken::from_bytes(&99usize.to_le_bytes()).unwrap(),
        range: env::VirtAddr::new(0x6000),
        out: env::VirtAddr::new(0x7001),
    };
    assert_eq!(call.pack(), [99, 0x6000, 0x7001, 0, 0, 0]);
    assert_eq!(decode(11, call.pack()), call);
}

#[test]
fn attach_passes_five_words_and_returns_two() {
    // range 指针 + team + placement 指针 + access + control。
    let regs = [0x4000, 3, 0x5000, 10, 1];
    let mut call =
        MemoryCall::from_wire((2usize << 32) | 7, &[0x4000, 3, 0x5000, 10, 1, 0]).unwrap();
    match call {
        MemoryCall::Attach {
            range,
            team,
            placement,
            access,
            control,
        } => {
            assert_eq!(range.get(), 0x4000);
            assert_eq!(team.get(), 3);
            assert_eq!(placement.get(), 0x5000);
            assert_eq!(access, 10);
            assert_eq!(control, ViewControl::SpaceMembers);
        }
        other => panic!("unexpected attach: {other:?}"),
    }
    assert_eq!(
        call.pack(),
        [regs[0], regs[1], regs[2], regs[3], regs[4], 0]
    );
    // 控制位只有 0/1 两支；其余非法。
    assert!(MemoryCall::from_wire((2usize << 32) | 7, &[0x4000, 3, 0x5000, 10, 2, 0]).is_err());
    call = decode(7, [0x4000, 0, 0, 2, 0, 0]);
    match call {
        MemoryCall::Attach { control, .. } => assert_eq!(control, ViewControl::Caller),
        other => panic!("unexpected attach: {other:?}"),
    }

    let (view, at) = <(ViewId, VirtAddr) as FromPair>::from_pair(0x1234, 0x5000);
    assert_eq!(view.get(), 0x1234);
    assert_eq!(at.get(), 0x5000);
}

#[test]
fn detach_and_set_access_use_the_same_selection() {
    let regs = [3, 9, 0, 0, 0, 0];
    let call = decode(8, regs);
    match call {
        MemoryCall::Detach {
            team,
            view,
            selection,
        } => {
            assert_eq!((team.get(), view.get()), (3, 9));
            assert_eq!(selection, ViewSelection::Whole);
        }
        other => panic!("unexpected detach: {other:?}"),
    }
    assert_eq!(call.pack(), regs);

    let regs = [3, 9, 1, 1, 2, 10];
    let call = decode(9, regs);
    match call {
        MemoryCall::SetAccess {
            selection, access, ..
        } => {
            assert_eq!(selection, ViewSelection::pages(1, 2).unwrap());
            assert_eq!(access, 10);
        }
        other => panic!("unexpected set_access: {other:?}"),
    }
    assert_eq!(call.pack(), regs);

    assert!(MemoryCall::from_wire((2usize << 32) | 8, &[3, 9, 0, 1, 0, 0]).is_err());
    assert!(MemoryCall::from_wire((2usize << 32) | 8, &[3, 9, 1, 0, 0, 0]).is_err());
    assert!(MemoryCall::from_wire((2usize << 32) | 8, &[3, 9, 1, usize::MAX, 2, 0]).is_err());
}

#[test]
fn range_types_use_checked_coordinates() {
    assert!(ReferenceRange::new(reference_of(5), 1, 2).is_some());
    assert!(ReferenceRange::new(reference_of(5), 0, 0).is_none());
    assert!(ReferenceRange::new(reference_of(5), usize::MAX, 2).is_none());
    assert!(ReferenceRange::new(reference_of(5), 0, usize::MAX).is_none());
    assert!(ViewSelection::pages(0, 0).is_none());
    assert!(ViewSelection::pages(usize::MAX, 2).is_none());
    assert!(Allocation::new(0, 4096).is_none());
    assert!(Allocation::new(usize::MAX, 4096).is_none());
}

#[test]
fn placement_descriptor_round_trips_and_rejects_bad_words() {
    let lowest = Placement::lowest(2 * env::PAGE_SIZE).expect("lowest");
    assert_eq!(lowest.at(), PlacementAt::Lowest);
    assert_eq!(lowest.align(), 2 * env::PAGE_SIZE);
    assert_eq!(lowest.encode(), [0, 0, 2 * env::PAGE_SIZE]);
    assert_eq!(Placement::decode(lowest.encode()), Ok(lowest));

    let exact = Placement::exact(VirtAddr::new(0x1_0000), 2 * env::PAGE_SIZE).expect("exact");
    assert_eq!(exact.encode(), [2, 0x1_0000, 2 * env::PAGE_SIZE]);
    assert_eq!(Placement::decode(exact.encode()), Ok(exact));

    let highest = Placement::decode([1, 0, env::PAGE_SIZE]).expect("highest");
    assert_eq!(highest.at(), PlacementAt::Highest);
    assert_eq!(Placement::decode(highest.encode()), Ok(highest));

    assert!(Placement::decode([3, 0, env::PAGE_SIZE]).is_err());
    assert!(Placement::decode([0, 0x1000, env::PAGE_SIZE]).is_err());
    assert!(Placement::decode([0, 0, 3000]).is_err());
    assert!(Placement::decode([0, 0, 2048]).is_err());
    assert!(Placement::decode([2, 0x1008, env::PAGE_SIZE]).is_err());
    assert!(Placement::exact(VirtAddr::new(0x1000), 2 * env::PAGE_SIZE).is_none());
}

/// 公开安全构造器不得因非法 `align` 而 panic（模运算前先验对齐）。
#[test]
fn placement_constructors_reject_bad_alignment_without_panicking() {
    for align in [0usize, 1, 2, 3, 3000, env::PAGE_SIZE - 1] {
        assert!(Placement::lowest(align).is_none(), "align {align}");
        assert!(
            Placement::exact(VirtAddr::new(0), align).is_none(),
            "align {align}"
        );
    }
    // 高位对齐：是 2 的幂就不因取模溢出而拒。
    let huge = 1usize << (usize::BITS - 1);
    assert_eq!(Placement::lowest(huge).map(Placement::align), Some(huge));
    assert_eq!(
        Placement::exact(VirtAddr::new(0), huge).map(Placement::align),
        Some(huge)
    );
    assert!(Placement::exact(VirtAddr::new(huge + 1), huge).is_none());
    // 定点必须同时满足页对齐与更高对齐；对齐理顺后 default `Highest` 仍可解。
    assert_eq!(
        Placement::decode([1, 0, 4 * env::PAGE_SIZE]).map(Placement::align),
        Ok(4 * env::PAGE_SIZE)
    );
}

#[test]
fn unpack_reports_overflow_past_the_argument_window() {
    assert_eq!(
        <usize as Wire>::unpack(&[0; 6], &mut 6),
        Err(Decode::Overflow)
    );
    assert_eq!(<usize as Wire>::unpack(&[0; 6], &mut 5), Ok(0));
}

#[test]
fn descriptors_use_fixed_width_little_endian_fields() {
    use env::wire::Field;
    assert_eq!(ReferenceRange::WIDTH, 24);
    assert_eq!(Placement::WIDTH, 24);
    let range = ReferenceRange::new(reference_of(7), 1, 2).unwrap();
    let mut bytes = [0; ReferenceRange::WIDTH];
    range.store(&mut bytes);
    for (actual, expected) in bytes.chunks_exact(8).zip([7u64, 1, 2]) {
        assert_eq!(actual, expected.to_le_bytes());
    }
    assert_eq!(ReferenceRange::fetch(&bytes), Some(range));
    assert_eq!(ReferenceRange::fetch(&bytes[..23]), None);
    assert_eq!(ReferenceRange::fetch(&[0; 25]), None);
    let placement = Placement::exact(VirtAddr::new(0x1_0000), 8192).unwrap();
    let mut bytes = [0; Placement::WIDTH];
    placement.store(&mut bytes);
    for (actual, expected) in bytes.chunks_exact(8).zip([2u64, 0x1_0000, 8192]) {
        assert_eq!(actual, expected.to_le_bytes());
    }
    assert_eq!(Placement::fetch(&bytes), Some(placement));
    bytes[..8].copy_from_slice(&3u64.to_le_bytes());
    assert_eq!(
        Placement::decode_bytes(&bytes),
        Err(env::PlacementError::Tag)
    );
    placement.store(&mut bytes);
    bytes[16..24].copy_from_slice(&0u64.to_le_bytes());
    assert_eq!(
        Placement::decode_bytes(&bytes),
        Err(env::PlacementError::Align)
    );
}

#[test]
fn space_ids_are_sendable_and_handles_reject_the_error_number_domain() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ViewId>();
    let max = isize::MAX as usize;
    assert_eq!(ViewId::from_bytes(&max.to_le_bytes()).unwrap().get(), max);
    assert_eq!(
        FlatReference::from_bytes(&max.to_le_bytes()).unwrap().get(),
        max
    );
    for raw in [max + 1, usize::MAX] {
        assert!(ViewId::from_bytes(&raw.to_le_bytes()).is_none());
        assert!(FlatReference::from_bytes(&raw.to_le_bytes()).is_none());
        assert_eq!(
            MemoryCall::from_wire((2usize << 32) | 6, &[raw, 0, 0, 0, 0, 0]),
            Err(Decode::Invalid)
        );
        assert_eq!(
            MemoryCall::from_wire((2usize << 32) | 8, &[0, raw, 0, 0, 0, 0]),
            Err(Decode::Invalid)
        );
    }
}
