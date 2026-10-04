use env::ledger::entry::{Entries, Header, VERSION};
use env::wire::{Span, store_tail};
use env::{Call, Entry, Name, Page, PieKind, PieToken, Trap};

fn encode(entries: &[Entry]) -> Vec<u8> {
    let mut bytes = vec![0; Header::LEN + entries.len() * Entry::LEN];
    let at = Header {
        version: VERSION,
        count: entries.len() as u32,
    }
    .store_at(&mut bytes, 0)
    .unwrap();
    assert_eq!(store_tail(&mut bytes, at, entries), Some(bytes.len()));
    bytes
}
fn entry(name: Name, token: usize) -> Entry {
    Entry::new(name, name.kind(), PieToken::mint(token))
}

#[test]
fn heterogeneous_directory_roundtrips() {
    let entries = [
        entry(Name::Trap(Trap::SupervisorExternal), 1),
        entry(Name::Trap(Trap::PageFault), 2),
        entry(Name::Call(Call::Build), 3),
        entry(Name::Call(Call::Doom), 7),
        entry(Name::Page(Page::Dtb), 4),
        entry(Name::Page(Page::Initrd), 5),
        entry(Name::Page(Page::Region(0x10000000)), 6),
    ];
    let bytes = encode(&entries);
    let ledger = Entries::new(&bytes).unwrap();
    assert_eq!(ledger.iter().collect::<Vec<_>>(), entries);
    assert_eq!(
        ledger.find(Name::Trap(Trap::PageFault)).unwrap().kind(),
        PieKind::Hole
    );
    assert_eq!(Entry::LEN, 32);
}
#[test]
fn directory_rejects_truncation_version_and_count_mismatch() {
    let bytes = encode(&[entry(Name::Call(Call::Build), 1)]);
    for len in 0..bytes.len() {
        assert!(Entries::new(&bytes[..len]).is_none());
    }
    let mut bad = bytes.clone();
    bad[0] = 99;
    assert!(Entries::new(&bad).is_none());
    let mut bad = bytes.clone();
    bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Entries::new(&bad).is_none());
    let mut bad = bytes;
    bad.push(0);
    assert!(Entries::new(&bad).is_none());
}
#[test]
fn directory_rejects_unknown_noncanonical_and_mistyped_records() {
    assert!(!entry(Name::Page(Page::Region(0)), 1).valid());
    let bytes = encode(&[entry(Name::Call(Call::Build), 1)]);
    for (offset, value) in [(8, 99), (9, 99), (10, 1), (16, 1), (24, 99), (25, 1)] {
        let mut bad = bytes.clone();
        bad[offset] = value;
        assert!(Entries::new(&bad).is_none(), "offset {offset}");
    }
    let wrong = Entry::new(
        Name::Trap(Trap::PageFault),
        PieKind::Nole,
        PieToken::mint(1),
    );
    assert!(Entries::new(&encode(&[wrong])).is_none());
    assert!(Entries::new(&encode(&[Entry::NONE])).is_none());
    assert!(Entries::new(&encode(&[entry(Name::Page(Page::Region(0)), 1)])).is_none());
}
#[test]
fn directory_rejects_duplicate_names() {
    let name = Name::Page(Page::Dtb);
    assert!(Entries::new(&encode(&[entry(name, 1), entry(name, 2)])).is_none());
    assert!(Entries::new(&encode(&[])).is_some());
}
