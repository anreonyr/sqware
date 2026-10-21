use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr};
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat};
use crate::core::memory::nest::Access;
use crate::core::work::pend::{HoldTable, SourceAuthorization};

pub fn ranges() {
    let flat = Flat::acquire(Allocation::new(8, PAGE_SIZE).unwrap()).unwrap();
    let whole = flat.whole();
    let left = whole
        .range(PageOffset::new(1), PageCount::new(3).unwrap())
        .unwrap();
    let right = whole
        .range(PageOffset::new(3), PageCount::new(3).unwrap())
        .unwrap();
    let prefix = whole
        .range(PageOffset::new(1), PageCount::new(1).unwrap())
        .unwrap();
    let edge = whole
        .range(PageOffset::new(6), PageCount::new(1).unwrap())
        .unwrap();
    assert!(!whole.is_pinned());
    let first = left.reside().unwrap();
    assert!(prefix.is_pinned());
    assert!(!edge.is_pinned());
    assert!(
        whole.materialized_frames().unwrap().is_empty(),
        "residency must remain lazy"
    );
    let second = right.reside().unwrap();
    drop(first);
    assert!(
        !prefix.is_pinned(),
        "one lease does not cover a different window"
    );
    assert!(right.is_pinned(), "independent overlapping lease survives");
    drop(second);
    assert!(!whole.is_pinned());
    let pages = left.materialize().unwrap();
    let address = pages.physical(PageOffset::ZERO).unwrap();
    assert!(left.is_pinned());
    assert!(!edge.is_pinned());
    let other = left.materialize().unwrap();
    assert_eq!(other.physical(PageOffset::ZERO).unwrap(), address);
    drop(pages);
    assert!(
        left.is_pinned(),
        "one Pages drop does not settle another lease"
    );
    drop(other);
    assert!(!whole.is_pinned());
    let logical = left.clone();
    assert!(
        !logical.is_pinned(),
        "FlatRange clones are not physical pins"
    );
}

pub fn retained_snapshot() {
    use crate::core::work::pend::{Geometry, IndependentHold};

    let flat = Flat::acquire(Allocation::new(2, PAGE_SIZE).unwrap()).unwrap();
    let whole = flat.whole();
    let source =
        SourceAuthorization::new(whole.clone(), Access::all(), env::TaskId::new(91)).unwrap();
    let table = HoldTable::new();
    let insert = || {
        // 独立持有 = 物理保留 + 它在来源记录上的登记：登记位先备，两者一起生。
        source.reserve_retention().unwrap();
        let window = Geometry::new(PageOffset::ZERO, whole.page_count()).unwrap();
        let residency = whole.reside().unwrap();
        table
            .insert(
                source.clone(),
                PageOffset::ZERO,
                whole.page_count(),
                Access::READ,
                Some(IndependentHold::new(
                    &source,
                    window,
                    residency,
                    env::TaskId::new(91),
                )),
                None,
            )
            .unwrap()
    };
    let id = insert();
    let snapshot = table.get(id).unwrap();
    assert!(whole.is_pinned());
    drop(table.remove(id).unwrap());
    assert!(table.get(id).is_none());
    assert!(
        whole.is_pinned(),
        "removing the table entry cannot end in-flight use"
    );
    drop(snapshot);
    assert!(!whole.is_pinned());
    let id = insert();
    let snapshot = table.get(id).unwrap();
    table.clear();
    assert!(
        whole.is_pinned(),
        "task table cleanup cannot end an outstanding snapshot"
    );
    drop(snapshot);
    assert!(!whole.is_pinned());
}

pub fn borrowed() {
    let source = Flat::from_region(unsafe {
        BorrowedRegion::new(PhysAddr::from_raw(0x2000), 2 * PAGE_SIZE).unwrap()
    })
    .unwrap();
    let range = source
        .range(PageIndex::new(1), PageCount::new(1).unwrap())
        .unwrap();
    let pages = range.materialize().unwrap();
    assert_eq!(pages.physical(PageOffset::ZERO).unwrap().as_usize(), 0x3000);
    assert!(range.is_pinned());
    assert!(
        range.materialized_frames().unwrap().is_empty(),
        "borrowed pages are not RAM slots"
    );
    drop(pages);
    assert!(!range.is_pinned());
}
