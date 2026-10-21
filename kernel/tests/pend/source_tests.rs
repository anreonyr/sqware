use alloc::sync::Arc;
use env::{PieFail, TaskId};

use crate::memory::address::{PageCount, PageOffset};
use crate::memory::flat::FlatRange;
use crate::memory::nest::Access;
use crate::pend::{HoldTable, SourceAuthorization};

fn source(owner: TaskId) -> Arc<SourceAuthorization> {
    SourceAuthorization::new(
        FlatRange::new(PageCount::new(8).unwrap()),
        Access::READ | Access::WRITE,
        owner,
    )
    .unwrap()
}

#[test]
fn creation_is_single_use_and_bound_to_one_source() {
    let owner = TaskId::new(101);
    let root = source(owner);
    let other = source(owner);
    let table = HoldTable::new();
    let id = table
        .insert(
            root.clone(),
            PageOffset::ZERO,
            PageCount::new(8).unwrap(),
            Access::READ,
            None,
            None,
        )
        .unwrap();
    let slice = table
        .insert(
            root.clone(),
            PageOffset::new(1),
            PageCount::new(2).unwrap(),
            Access::READ,
            None,
            None,
        )
        .unwrap();
    assert!(Arc::ptr_eq(
        table.get(id).unwrap().source(),
        table.get(slice).unwrap().source()
    ));
    assert!(
        HoldTable::new().get(id).is_none(),
        "a foreign table has no such reference"
    );
    assert!(root.private_owner(owner));
    assert!(!root.private_owner(TaskId::new(102)));
    root.claim().unwrap();
    assert_eq!(root.claim(), Err(PieFail::Busy));
    root.consume();
    assert_eq!(
        table.get(slice).unwrap().source().claim(),
        Err(PieFail::Denied)
    );
    other
        .claim()
        .expect("a distinct source has its own qualification");
    other.release_claim();
    table.clear();
}

#[test]
fn failed_claim_rolls_the_creation_right_back() {
    let root = source(TaskId::new(101));
    root.claim().unwrap();
    assert_eq!(root.claim(), Err(PieFail::Busy));
    root.release_claim();
    root.claim()
        .expect("failed transaction restores availability");
    root.consume();
    assert_eq!(root.claim(), Err(PieFail::Denied));
}

#[test]
fn committed_claim_cannot_restore_consumed_creation() {
    let root = source(TaskId::new(101));
    root.claim().unwrap();
    root.consume();
    root.release_claim();
    assert_eq!(root.claim(), Err(PieFail::Denied));
}

#[test]
fn retained_source_and_live_record_cannot_extend_creation_qualification() {
    let owner = TaskId::new(101);
    let root = source(owner);
    let table = HoldTable::new();
    let live_record = (Arc::new(crate::memory::nest::View), root.clone());
    let count = PageCount::new(8).unwrap();
    let ordinary = table
        .insert(
            root.clone(),
            PageOffset::ZERO,
            count,
            Access::READ,
            None,
            None,
        )
        .unwrap();
    let retained = table
        .insert(
            root.clone(),
            PageOffset::ZERO,
            count,
            Access::READ,
            root.own_range().map(|range| {
                root.reserve_retention().unwrap();
                let window = crate::pend::Geometry::new(PageOffset::ZERO, count).unwrap();
                crate::pend::IndependentHold::new(&root, window, range.reside().unwrap(), owner)
            }),
            None,
        )
        .unwrap();
    table.remove(ordinary).unwrap();
    assert_eq!(root.claim(), Err(PieFail::Denied));
    assert!(table.get(retained).unwrap().resolve().is_some());
    assert!(Arc::ptr_eq(&live_record.1, &root));
    table.remove(retained).unwrap();
    assert_eq!(root.claim(), Err(PieFail::Denied));
}

#[test]
fn identity_exhaustion_stops_before_the_error_domain_without_wraparound() {
    use core::sync::atomic::{AtomicUsize, Ordering};
    let max = isize::MAX as usize;
    let next = AtomicUsize::new(max - 1);
    assert_eq!(crate::serial::next_id(&next), Some(max - 1));
    assert_eq!(crate::serial::next_id(&next), Some(max));
    for _ in 0..3 {
        assert_eq!(crate::serial::next_id(&next), None);
        assert_eq!(next.load(Ordering::Relaxed), max + 1);
    }
    let zero = AtomicUsize::new(0);
    assert_eq!(crate::serial::next_id(&zero), None);
    assert_eq!(zero.load(Ordering::Relaxed), 0);
}
