use alloc::vec::Vec;

use crate::core::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::mode;
use crate::core::memory::manager::table::{MapError, TableNode};
use crate::core::memory::{PAGE_SHIFT, PAGE_SIZE};

pub(crate) fn large_pages_accept() {
    const MEGA: usize = 1 << 21;
    const GIGA: usize = 1 << 30;
    let va = VirtAddr::wrap(GIGA);
    let pa = PhysAddr::from_raw(2 * GIGA);
    let rw = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    let ro = rw - PteFlags::W;
    let geo = mode::geometry(mode::mode());
    let levels = geo.levels as usize;
    let mut root = TableNode::root().expect("large page root");
    root.prepare_range_geometry(va, pa, GIGA, geo)
        .expect("gigapage map");
    root.map_range_prepared_geometry(va, pa, GIGA, rw, geo);
    assert_eq!(root.count(), levels - 2);
    assert_eq!(root.leaf_ref_geometry(va, geo).unwrap().1, 2);
    for offset in [0, PAGE_SIZE, MEGA + PAGE_SIZE, GIGA - PAGE_SIZE] {
        let point = va + offset;
        assert_eq!(root.walk_ref_geometry(point, geo).unwrap().0, pa + offset);
        let physical_root = PhysAddr::from_raw(root.ppn() << PAGE_SHIFT);
        assert_eq!(
            TableNode::walk_raw(physical_root, point, |_| true)
                .unwrap()
                .0,
            pa + offset
        );
    }
    assert_eq!(
        root.prepare_range_geometry(va + PAGE_SIZE, pa, PAGE_SIZE, geo),
        Err(MapError::AlreadyMapped)
    );
    let count = root.count();
    {
        let guard = crate::core::memory::allocator::NoAllocation::enter();
        root.protect_permissions_geometry(va, GIGA, ro, geo)
            .expect("whole gigapage protect");
        root.protect_permissions_geometry(va, GIGA, PteFlags::R | PteFlags::W, geo)
            .unwrap();
        assert_eq!(root.count(), count);
        drop(guard);
    }
    let point = va + MEGA + PAGE_SIZE;
    let mut allocations = 0;
    let result = root.prepare_with(
        point,
        PAGE_SIZE,
        &mut || {
            allocations += 1;
            if allocations == 2 {
                Err(MapError::OutOfMemory)
            } else {
                TableNode::leaf()
            }
        },
        geo,
    );
    assert_eq!(result, Err(MapError::OutOfMemory));
    assert_eq!(allocations, 2);
    assert_eq!(root.count(), count + 1);
    for offset in [0, MEGA, MEGA + PAGE_SIZE, GIGA - PAGE_SIZE] {
        let (physical, flags) = root.walk_ref_geometry(va + offset, geo).unwrap();
        assert_eq!(physical, pa + offset);
        assert_eq!(flags.bits(), rw.bits());
    }
    root.protect_permissions_geometry(point, PAGE_SIZE, ro, geo)
        .expect("retry partial protect");
    assert_eq!(root.count(), count + 2);
    assert_eq!(root.leaf_ref_geometry(point, geo).unwrap().1, 0);
    assert_eq!(root.leaf_ref_geometry(va, geo).unwrap().1, 1);
    assert!(
        !root
            .walk_ref_geometry(point, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    for neighbor in [point - PAGE_SIZE, point + PAGE_SIZE] {
        assert!(
            root.walk_ref_geometry(neighbor, geo)
                .unwrap()
                .1
                .contains(PteFlags::W)
        );
    }
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(point, PAGE_SIZE, geo)
        .expect("partial unmap");
    root.unmap_keep_tables_geometry(point, PAGE_SIZE, geo)
        .expect("partial unmap");
    root.reclaim_empty_tables(&mut retired);
    assert!(root.walk_ref_geometry(point, geo).is_err());
    root.prepare_range_geometry(point, pa + MEGA + PAGE_SIZE, PAGE_SIZE, geo)
        .expect("refill small hole");
    root.map_range_prepared_geometry(point, pa + MEGA + PAGE_SIZE, PAGE_SIZE, rw, geo);
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    let guard = crate::core::memory::allocator::NoAllocation::enter();
    root.prepare_geometry(va, GIGA, geo)
        .expect("whole gigapage removal");
    root.unmap_keep_tables_geometry(va, GIGA, geo)
        .expect("whole gigapage removal");
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);
    drop(guard);

    let start = va - PAGE_SIZE;
    root.prepare_range_geometry(start, pa - PAGE_SIZE, MEGA + 2 * PAGE_SIZE, geo)
        .expect("mixed map");
    root.map_range_prepared_geometry(start, pa - PAGE_SIZE, MEGA + 2 * PAGE_SIZE, rw, geo);
    for (point, expected) in [(start, 0), (va, 1), (va + MEGA, 0)] {
        assert_eq!(root.leaf_ref_geometry(point, geo).unwrap().1, expected);
    }
    root.protect_permissions_geometry(va + MEGA - PAGE_SIZE, 2 * PAGE_SIZE, ro, geo)
        .expect("cross leaf boundary");
    assert!(
        !root
            .walk_ref_geometry(va + MEGA - PAGE_SIZE, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(
        !root
            .walk_ref_geometry(va + MEGA, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(
        root.walk_ref_geometry(va + MEGA - 2 * PAGE_SIZE, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(start, MEGA + 2 * PAGE_SIZE, geo)
        .unwrap();
    root.unmap_keep_tables_geometry(start, MEGA + 2 * PAGE_SIZE, geo)
        .unwrap();
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);

    root.prepare_range_geometry(va + PAGE_SIZE, pa + PAGE_SIZE, PAGE_SIZE, geo)
        .unwrap();
    root.map_range_prepared_geometry(va + PAGE_SIZE, pa + PAGE_SIZE, PAGE_SIZE, rw, geo);
    root.prepare_range_geometry(va, pa, PAGE_SIZE, geo).unwrap();
    root.map_range_prepared_geometry(va, pa, PAGE_SIZE, rw, geo);
    root.prepare_range_geometry(
        va + 2 * PAGE_SIZE,
        pa + 2 * PAGE_SIZE,
        MEGA - 2 * PAGE_SIZE,
        geo,
    )
    .unwrap();
    root.map_range_prepared_geometry(
        va + 2 * PAGE_SIZE,
        pa + 2 * PAGE_SIZE,
        MEGA - 2 * PAGE_SIZE,
        rw,
        geo,
    );
    assert_eq!(root.leaf_ref_geometry(va, geo).unwrap().1, 0);
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(va, MEGA, geo).unwrap();
    root.unmap_keep_tables_geometry(va, MEGA, geo).unwrap();
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);

    let top = VirtAddr::wrap(usize::MAX - MEGA + 1);
    root.prepare_range_geometry(top, pa, MEGA, geo)
        .expect("top megapage");
    root.map_range_prepared_geometry(top, pa, MEGA, rw, geo);
    root.protect_permissions_geometry(top + (MEGA - PAGE_SIZE), PAGE_SIZE, ro, geo)
        .unwrap();
    assert_eq!(
        root.walk_ref_geometry(VirtAddr::wrap(usize::MAX), geo)
            .unwrap()
            .0,
        pa + MEGA - PAGE_SIZE
    );
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(top, MEGA, geo)
        .expect("top megapage removal");
    root.unmap_keep_tables_geometry(top, MEGA, geo)
        .expect("top megapage removal");
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);
}
