//! Filter an already built acceptance image; keep the actual ELF and manifest codec.

use env::ledger::manifest::{Entries, pack};

const UNITS: &[&str] = &[
    "operator", "identity", "subject", "member", "probe-bound", "probe-coalition",
    "probe-control", "probe-denied", "system",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let source = args.next().expect("source initrd");
    let destination = args.next().expect("destination initrd");
    assert!(args.next().is_none());
    let blob = std::fs::read(source).expect("read acceptance initrd");
    let entries: Vec<_> = Entries::new(&blob).expect("valid acceptance header")
        .map(|entry| entry.expect("valid acceptance entry"))
        .filter(|entry| UNITS.contains(&entry.name)).collect();
    assert_eq!(entries.len(), UNITS.len(), "all fixture programs must exist");
    for name in UNITS {
        assert_eq!(entries.iter().filter(|entry| entry.name == *name).count(), 1);
    }
    let boot = entries.iter().position(|entry| entry.name == "system").unwrap();
    let items: Vec<_> = entries.iter().map(|entry| (entry.kind, entry.name, entry.elf)).collect();
    let filtered = pack(&items, boot).expect("valid fixture manifest");
    std::fs::write(destination, filtered).expect("write fixture initrd");
    println!("identity smoke: {} actual programs; no Hub or driver", entries.len());
}
