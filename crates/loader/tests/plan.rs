use env::ledger::capsule::{Capsule, PAGE};

fn image(flags: u32) -> Vec<u8> {
    let mut bytes = vec![0; PAGE + 4];
    bytes[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&243u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x10000u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&flags.to_le_bytes());
    bytes[72..80].copy_from_slice(&(PAGE as u64).to_le_bytes());
    bytes[80..88].copy_from_slice(&0x10000u64.to_le_bytes());
    bytes[96..104].copy_from_slice(&4u64.to_le_bytes());
    bytes[104..112].copy_from_slice(&(2 * PAGE as u64).to_le_bytes());
    bytes[112..120].copy_from_slice(&(PAGE as u64).to_le_bytes());
    bytes[PAGE..].copy_from_slice(&[0x13, 0, 0, 0]);
    bytes
}

#[test]
fn executable_tail_is_payload_and_entry_is_original_content() {
    let bytes = image(5);
    let plan = loader::parse(&bytes).unwrap();
    assert_eq!(plan.regions[0].data_size, 2 * PAGE);
    let mut packed = loader::capsule(&bytes).unwrap();
    let capsule = Capsule::parse(&packed).unwrap();
    let region = capsule.region(0).unwrap();
    assert_eq!(region.flags, 10);
    assert_eq!(region.data_pages, region.pages);
    assert_eq!(
        &packed[region.payload..region.payload + 4],
        &[0x13, 0, 0, 0]
    );
    assert!(packed[region.payload + 4..].iter().all(|&byte| byte == 0));
    packed[48..56].copy_from_slice(&1u64.to_le_bytes());
    assert!(Capsule::parse(&packed).is_none());
    let mut bytes = bytes;
    bytes[24..32].copy_from_slice(&0x11000u64.to_le_bytes());
    assert!(matches!(loader::parse(&bytes), Err(loader::Error::Entry)));
}

#[test]
fn malformed_permissions_ranges_and_layout_are_rejected() {
    assert!(matches!(
        loader::parse(&image(7)),
        Err(loader::Error::Permissions)
    ));
    let mut bytes = image(5);
    bytes.pop();
    assert!(matches!(loader::parse(&bytes), Err(loader::Error::Range)));
    let mut packed = loader::capsule(&image(5)).unwrap();
    packed[56..64].copy_from_slice(&0u64.to_le_bytes());
    assert!(Capsule::parse(&packed).is_none());
    let mut bytes = image(5);
    bytes[80..88].copy_from_slice(&0x10002u64.to_le_bytes());
    assert!(matches!(loader::parse(&bytes), Err(loader::Error::Format)));
}

#[test]
fn readonly_and_writable_tails_keep_their_final_permissions() {
    for permissions in [4u32, 6] {
        let mut bytes = image(5);
        bytes.resize(2 * PAGE + 19, 0);
        bytes[56..58].copy_from_slice(&2u16.to_le_bytes());
        let at = 120;
        bytes[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&permissions.to_le_bytes());
        for (field, value) in [
            (8, (2 * PAGE + 16) as u64),
            (16, 0x20010),
            (32, 3),
            (40, (2 * PAGE + 24) as u64),
            (48, PAGE as u64),
        ] {
            bytes[at + field..at + field + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[2 * PAGE + 16..].copy_from_slice(b"abc");
        let plan = loader::parse(&bytes).unwrap();
        let data = &plan.regions[1];
        assert_eq!(
            (data.va, data.size, data.data_size, data.prefix),
            (0x20000, 3 * PAGE, PAGE, 16)
        );
        let packed = loader::capsule(&bytes).unwrap();
        let capsule = Capsule::parse(&packed).unwrap();
        let region = capsule.region(1).unwrap();
        assert_eq!(region.flags, data.flags);
        assert_eq!((region.pages, region.data_pages), (3, 1));
        assert!(
            packed[region.payload..region.payload + 16]
                .iter()
                .all(|&byte| byte == 0)
        );
        assert_eq!(&packed[region.payload + 16..region.payload + 19], b"abc");
        assert!(
            packed[region.payload + 19..region.payload + PAGE]
                .iter()
                .all(|&byte| byte == 0)
        );
    }
}
