use env::ProgramKind;
use env::ledger::manifest::pack;

#[path = "fixtures/accounts.rs"]
mod accounts;
use accounts::Accounts;
#[path = "../../../programs/src/boot/catalog.rs"]
mod catalog;
use catalog::Catalog;

#[test]
fn catalog_reads_explicit_deployment_and_selected_entry() {
    let blob = pack(&[
        (ProgramKind::Supervisor, "accept-system", b"accept elf"),
        (ProgramKind::Supervisor, "system", b"system elf"),
        (ProgramKind::User, "worker", b"worker elf"),
    ], 1, "iommu-product-fault").unwrap();
    let blob = Box::leak(blob.into_boxed_slice());
    let catalog = Catalog::of_boot(&Accounts::new(blob)).unwrap();
    assert_eq!(catalog.deployment(), "iommu-product-fault");
    assert_eq!(catalog.entry().name, "system");
    assert_eq!(catalog.entry().kind, ProgramKind::Supervisor);
    assert_eq!(catalog.find("worker").unwrap().elf, b"worker elf");
    assert!(catalog.find("absent").is_none());
}

#[test]
fn catalog_rejects_truncation_anywhere_in_the_manifest() {
    let blob = pack(&[
        (ProgramKind::Supervisor, "system", b"system elf"),
        (ProgramKind::User, "worker", b"worker elf"),
    ], 0, "product").unwrap();
    for len in 0..blob.len() {
        assert!(Catalog::new(&blob[..len]).is_none(), "truncated at {len}");
    }
    assert!(Catalog::new(&blob).is_some());
}

#[test]
fn catalog_rejects_invalid_records_after_a_valid_selected_entry() {
    let mut blob = pack(&[
        (ProgramKind::Supervisor, "system", b"system elf"),
        (ProgramKind::User, "worker", b"worker elf"),
    ], 0, "product").unwrap();
    let second = 24 + "product".len() + 12 + "system".len() + b"system elf".len();
    blob[second..second + 4].copy_from_slice(&9u32.to_le_bytes());
    assert!(Catalog::new(&blob).is_none());
}
