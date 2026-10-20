use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageIndex, PageOffset, VirtAddr};
use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
use crate::core::memory::flat::{Allocation, Flat, FlatError};
use crate::core::memory::nest::identity::{fail_shootdown, restore_shootdown};
use crate::core::memory::nest::source_views;
use crate::core::memory::nest::table::TABLE_LIVE;
use crate::core::memory::nest::{Access, NestError};
use crate::core::memory::transfer::{Transfer, TransferError};
use crate::core::work::room::messenger;
use crate::core::work::unit::space::SpaceBuilder;

use super::support;
use super::transfer::{fill_pattern, pattern_holds};

const SOURCE: VirtAddr = VirtAddr::wrap(0x6800_0000);
const TARGET: VirtAddr = VirtAddr::wrap(0x6800_1000);

pub fn closed_view_does_not_block_transfer() {
    drop(SpaceBuilder::user().build().unwrap());
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let pages = frame_occupied() - block_occupied();
    let sites = messenger::site_count();
    let records = source_views::len();
    let held = {
        let source = Flat::acquire(Allocation::new(1, PAGE_SIZE).unwrap())
            .unwrap()
            .whole();
        let target = Flat::acquire(Allocation::new(1, PAGE_SIZE).unwrap())
            .unwrap()
            .whole();
        let physical = source
            .materialize()
            .unwrap()
            .physical(PageOffset::ZERO)
            .unwrap();
        let held = Arc::downgrade(&source.materialized_frames().unwrap()[0].1);
        fill_pattern(physical);
        let rw = Access::READ | Access::WRITE;
        let live = SpaceBuilder::user().build().unwrap();
        let reservation = support::reserve(&live, SOURCE, PAGE_SIZE).unwrap();
        let live_view = live
            .nest
            .attach(reservation, source.clone(), rw, rw)
            .unwrap();
        let closed = SpaceBuilder::user().build().unwrap();
        let reservation = support::reserve(&closed, SOURCE, PAGE_SIZE).unwrap();
        let closed_view = closed
            .nest
            .attach(reservation, source.clone(), rw, rw)
            .unwrap();
        let asid = closed.nest.asid();

        fail_shootdown(asid);
        assert_eq!(closed.nest.close(), Err(NestError::Shootdown));
        assert!(!closed.nest.settled(closed_view.identity()));
        let failed = Transfer::new(
            source.clone(),
            PageOffset::ZERO,
            target.clone(),
            PageOffset::ZERO,
        )
        .unwrap()
        .execute();
        assert_eq!(failed.result, Err(TransferError::Busy));
        assert!(target.materialized_frames().unwrap().is_empty());
        assert!(Arc::ptr_eq(
            &source.materialized_frames().unwrap()[0].1,
            &held.upgrade().unwrap()
        ));
        assert!(pattern_holds(physical));
        drop(failed);

        restore_shootdown(asid);
        closed.nest.close().unwrap();
        assert_eq!(closed_view.frame_count(), 0);
        assert!(closed_view.owner().is_none());
        assert!(!closed.nest.retired());
        assert!(closed.nest.table_count() > 0);
        let pressings = source_views::pressings_over(&source, PageIndex::ZERO).unwrap();
        let pressing = pressings
            .iter()
            .find(|pressing| Arc::ptr_eq(&pressing.view, &closed_view))
            .expect("the closed View remains a real transfer candidate");
        assert!(Arc::ptr_eq(pressing.nest.as_ref().unwrap(), &closed.nest));
        assert_eq!(pressing.address, SOURCE);
        drop(pressings);

        let moved = Transfer::new(
            source.clone(),
            PageOffset::ZERO,
            target.clone(),
            PageOffset::ZERO,
        )
        .unwrap()
        .execute();
        assert_eq!(
            moved.result,
            Ok(()),
            "a reliably closed View without published PTEs must not pin the source"
        );
        assert!(moved.affected.iter().any(|view| {
            view.upgrade()
                .is_some_and(|view| Arc::ptr_eq(&view, &closed_view))
        }));
        assert!(matches!(source.materialize(), Err(FlatError::Missing)));
        assert!(Arc::ptr_eq(
            &target.materialized_frames().unwrap()[0].1,
            &held.upgrade().unwrap()
        ));
        assert!(pattern_holds(physical));
        assert!(live.nest.translate(SOURCE).is_none());
        assert!(closed.nest.translate(SOURCE).is_none());
        assert_eq!(live_view.frame_count(), 0);
        assert!(!closed.nest.retired());

        let reservation = support::reserve(&live, TARGET, PAGE_SIZE).unwrap();
        let target_view = live.nest.attach(reservation, target, rw, rw).unwrap();
        assert_eq!(live.nest.translate(TARGET).unwrap().0, physical);
        assert_eq!(target_view.frame_count(), 1);
        assert!(pattern_holds(physical));
        closed.retire().unwrap();
        assert!(closed.nest.retired());
        held
    };
    assert!(held.upgrade().is_none());
    drop(held);
    assert_eq!(source_views::len(), records);
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(frame_occupied() - block_occupied(), pages);
}
