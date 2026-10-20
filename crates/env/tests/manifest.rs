use env::ProgramKind;
use env::ledger::manifest::{Entries, MAX_PROGRAMS, Malformed, pack};

#[test]
fn deployment_and_entry_survive_iteration_and_capsule_relocation() {
    let items: &[(ProgramKind, &str, &[u8])] = &[
        (ProgramKind::User, "worker", b"worker elf"),
        (ProgramKind::Supervisor, "system", b"system elf"),
    ];
    let mut blob = pack(items, 1, "iommu-product-fault").unwrap();
    let offset = u32::from_le_bytes(blob[..4].try_into().unwrap()) as usize;
    let len = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
    assert_eq!(&blob[offset..offset + len], items[1].2);
    blob[..4].copy_from_slice(&4096u32.to_le_bytes());
    blob[4..8].copy_from_slice(&128u32.to_le_bytes());

    let mut entries = Entries::new(&blob).unwrap();
    assert_eq!(entries.deployment(), "iommu-product-fault");
    assert_eq!(entries.entry().unwrap().name, "system");
    assert_eq!(entries.next().unwrap().unwrap().name, "worker");
    assert_eq!(entries.next().unwrap().unwrap().name, "system");
    assert!(entries.next().is_none());
    assert_eq!(entries.entry().unwrap().elf, b"system elf");
}

#[test]
fn pack_rejects_invalid_selection_and_records() {
    let good = [(ProgramKind::Supervisor, "system", b"elf".as_slice())];
    assert!(pack(&[], 0, "product").is_none());
    assert!(pack(&good, 1, "product").is_none());
    assert!(pack(&good, usize::MAX, "product").is_none());
    assert!(pack(&good, 0, "").is_none());
    assert!(pack(&good, 0, &"x".repeat(33)).is_none());
    assert!(pack(&good.repeat(MAX_PROGRAMS + 1), 0, "product").is_none());
    assert!(pack(&[(ProgramKind::User, "", b"elf")], 0, "product").is_none());
    assert!(pack(&[(ProgramKind::User, "system", b"")], 0, "product").is_none());
    assert!(pack(&good, 0, &"x".repeat(32)).is_some());
    assert!(pack(&good.repeat(MAX_PROGRAMS), MAX_PROGRAMS - 1, "product").is_some());
}

#[test]
fn reader_rejects_missing_old_or_malformed_deployment_headers() {
    let good = pack(&[(ProgramKind::Supervisor, "system", b"elf")], 0, "product").unwrap();
    for len in 0..31 {
        assert!(Entries::new(&good[..len]).is_none(), "header truncated at {len}");
    }
    let mut old = vec![0; 8];
    old.extend_from_slice(&1u32.to_le_bytes());
    old.extend_from_slice(&good[31..]);
    assert!(Entries::new(&old).is_none(), "anonymous legacy headers must not be inferred");
    for (offset, value) in [(8, 0), (12, 0), (12, 97), (16, 1), (16, u32::MAX), (20, 0), (20, 33)] {
        let mut bad = good.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Entries::new(&bad).is_none(), "invalid header at {offset}: {value}");
    }
    let mut bad = good;
    bad[24] = 0xff;
    assert!(Entries::new(&bad).is_none());
}

#[test]
fn reader_reports_missing_or_malformed_selected_image() {
    let good = pack(&[(ProgramKind::Supervisor, "system", b"elf")], 0, "product").unwrap();
    for len in 31..good.len() {
        let entries = Entries::new(&good[..len]).unwrap();
        assert_eq!(entries.entry(), Err(Malformed), "entry truncated at {len}");
    }
    let mut bad = good;
    bad[31..35].copy_from_slice(&99u32.to_le_bytes());
    assert_eq!(Entries::new(&bad).unwrap().entry(), Err(Malformed));
}
