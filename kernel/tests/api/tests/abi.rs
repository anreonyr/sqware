use kernel_test_api::{device_confirmation, memory_transfer_fresh, memory_transfer_race};

#[test]
fn fresh_and_race_wire_layouts_match_frozen_golden_snapshots() {
    assert_eq!(memory_transfer_fresh::VA, 0x00F0_0000);
    assert_eq!(memory_transfer_fresh::WORDS, 7);
    assert_eq!(memory_transfer_fresh::TAG, 0x4652_4553_4800_0001);
    assert_eq!(memory_transfer_fresh::RELEASE, 1);
    assert_eq!(memory_transfer_fresh::VERDICT_NONE, 0xFFFF_FFFF_FFFF_FFFF);

    let fresh_offsets = [
        ("TAG", memory_transfer_fresh::off::TAG, 0usize),
        ("MODE", memory_transfer_fresh::off::MODE, 8),
        ("START", memory_transfer_fresh::off::START, 16),
        ("VIEW", memory_transfer_fresh::off::VIEW, 24),
        ("STATE", memory_transfer_fresh::off::STATE, 32),
        ("READY", memory_transfer_fresh::off::READY, 40),
        ("VERDICT", memory_transfer_fresh::off::VERDICT, 48),
    ];
    for (name, actual, expected) in fresh_offsets {
        assert_eq!(actual, expected, "fresh offset {name}");
    }

    let fresh_modes = [
        ("OFF", memory_transfer_fresh::mode::OFF, 0u64),
        ("AWAIT", memory_transfer_fresh::mode::AWAIT, 1),
        ("LATE", memory_transfer_fresh::mode::LATE, 2),
    ];
    for (name, actual, expected) in fresh_modes {
        assert_eq!(actual, expected, "fresh mode {name}");
    }

    let fresh_states = [
        ("IDLE", memory_transfer_fresh::state::IDLE, 0u64),
        ("PREPARED", memory_transfer_fresh::state::PREPARED, 1),
        ("ARMED", memory_transfer_fresh::state::ARMED, 2),
        ("TIMEOUT", memory_transfer_fresh::state::TIMEOUT, 3),
    ];
    for (name, actual, expected) in fresh_states {
        assert_eq!(actual, expected, "fresh state {name}");
    }

    assert_eq!(memory_transfer_race::VA, 0x00F0_0000);
    assert_eq!(memory_transfer_race::WORDS, 20);
    assert_eq!(memory_transfer_race::TAG, 0x5241_4345_0000_0001);
    assert_eq!(memory_transfer_race::RELEASE, 1);
    assert_eq!(memory_transfer_race::NONE, 0xFFFF_FFFF_FFFF_FFFF);

    let race_offsets = [
        ("TAG", memory_transfer_race::off::TAG, 0usize),
        ("MODE", memory_transfer_race::off::MODE, 8),
        ("START", memory_transfer_race::off::START, 16),
        ("STATE", memory_transfer_race::off::STATE, 24),
        ("READY", memory_transfer_race::off::READY, 32),
        ("PEER", memory_transfer_race::off::PEER, 40),
        ("VA0", memory_transfer_race::off::VA0, 48),
        ("VA1", memory_transfer_race::off::VA1, 56),
        ("READ0", memory_transfer_race::off::READ0, 64),
        ("READ1", memory_transfer_race::off::READ1, 72),
        ("FOREIGN", memory_transfer_race::off::FOREIGN, 80),
        ("PAUSE", memory_transfer_race::off::PAUSE, 88),
        ("GO", memory_transfer_race::off::GO, 96),
        ("RELEASED", memory_transfer_race::off::RELEASED, 104),
        ("PRESS_BEFORE", memory_transfer_race::off::PRESS_BEFORE, 112),
        ("PRESS_AFTER", memory_transfer_race::off::PRESS_AFTER, 120),
        ("PEER_BEFORE", memory_transfer_race::off::PEER_BEFORE, 128),
        ("PEER_AFTER", memory_transfer_race::off::PEER_AFTER, 136),
        ("RESUMED", memory_transfer_race::off::RESUMED, 144),
        ("SOURCE_TOKEN", memory_transfer_race::off::SOURCE_TOKEN, 152),
    ];
    for (name, actual, expected) in race_offsets {
        assert_eq!(actual, expected, "race offset {name}");
    }

    let race_modes = [
        ("OFF", memory_transfer_race::mode::OFF, 0u64),
        ("FAULT", memory_transfer_race::mode::FAULT, 1),
        ("TEARDOWN", memory_transfer_race::mode::TEARDOWN, 2),
    ];
    for (name, actual, expected) in race_modes {
        assert_eq!(actual, expected, "race mode {name}");
    }

    let race_states = [
        ("IDLE", memory_transfer_race::state::IDLE, 0u64),
        ("PREPARED", memory_transfer_race::state::PREPARED, 1),
        ("ARMED", memory_transfer_race::state::ARMED, 2),
        ("TIMEOUT", memory_transfer_race::state::TIMEOUT, 3),
        ("PAUSED", memory_transfer_race::state::PAUSED, 4),
    ];
    for (name, actual, expected) in race_states {
        assert_eq!(actual, expected, "race state {name}");
    }
}

#[test]
fn dma_confirmation_command_matches_frozen_64_byte_little_endian_frame() {
    assert_eq!(device_confirmation::TAG, b"DMAFAIL1");
    assert_eq!(device_confirmation::LEN, 64);
    assert_eq!(device_confirmation::REFERENCE_RELEASE, 0x100);
    assert_eq!(device_confirmation::BEFORE_DEVICE_STOP, 0x101);

    let enabled = device_confirmation::command(
        true,
        0x1122_3344_5566_7788,
        0x99AA_BBCC_DDEE_FF00,
        0x0102_0304_0506_0708,
        0x1112_1314_1516_1718,
        0x2122_2324_2526_2728,
        0x3132_3334_3536_3738,
    );
    let expected_enabled = [
        b'D', b'M', b'A', b'F', b'A', b'I', b'L', b'1',
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        0x00, 0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99,
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
        0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11,
        0x28, 0x27, 0x26, 0x25, 0x24, 0x23, 0x22, 0x21,
        0x38, 0x37, 0x36, 0x35, 0x34, 0x33, 0x32, 0x31,
    ];
    assert_eq!(enabled, expected_enabled);

    let disabled = device_confirmation::command(
        false,
        0x8877_6655_4433_2211,
        0x0102_0304_0506_0708,
        0xA1A2_A3A4_A5A6_A7A8,
        0xB1B2_B3B4_B5B6_B7B8,
        0xC1C2_C3C4_C5C6_C7C8,
        0xD1D2_D3D4_D5D6_D7D8,
    );
    let expected_disabled = [
        b'D', b'M', b'A', b'F', b'A', b'I', b'L', b'1',
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
        0xA8, 0xA7, 0xA6, 0xA5, 0xA4, 0xA3, 0xA2, 0xA1,
        0xB8, 0xB7, 0xB6, 0xB5, 0xB4, 0xB3, 0xB2, 0xB1,
        0xC8, 0xC7, 0xC6, 0xC5, 0xC4, 0xC3, 0xC2, 0xC1,
        0xD8, 0xD7, 0xD6, 0xD5, 0xD4, 0xD3, 0xD2, 0xD1,
    ];
    assert_eq!(disabled, expected_disabled);

    assert_eq!(enabled[8], 1);
    assert_eq!(disabled[8], 0);
}
