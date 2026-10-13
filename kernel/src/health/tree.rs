use alloc::sync::Arc;
use alloc::vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::Frame;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::{BorrowedRegion, Tree};

pub fn manager_tree() {
    const BASE: usize = 0x6000_0000;
    let frames = vec![
        Frame::allocate().expect("tree: first frame"),
        Frame::allocate().expect("tree: second frame"),
    ];
    unsafe { core::ptr::write_volatile(frames[0].as_mut_ptr(), 0x5a) };
    let first = Tree::new().expect("tree: first root");
    assert_ne!(first.asid(), crate::memory::manager::asid::Asid::kernel());
    assert_ne!(first.root_ppn(), 0);
    assert!(first.geometry().levels >= 3);
    let first_map = first
        .map_frames(
            VirtAddr::from_raw(BASE),
            frames.clone(),
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: map first root");
    assert!(!first.has_pending_invalidation());
    assert!(first.pending_maps().unwrap().is_empty());
    first.audit();
    let second = Tree::new().expect("tree: second root");
    let second_map = second
        .map_frames(
            VirtAddr::from_raw(BASE),
            frames.clone(),
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: map second root");

    assert_eq!(
        unsafe { core::ptr::read_volatile(frames[0].as_ptr()) },
        0x5a
    );
    assert!(Arc::ptr_eq(
        &first.find(VirtAddr::from_raw(BASE)).unwrap(),
        &first_map
    ));
    assert_eq!(
        first.translate(VirtAddr::from_raw(BASE + 37)).unwrap().0,
        crate::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address() + 37),
    );
    assert!(
        first
            .translate(VirtAddr::from_raw(BASE))
            .unwrap()
            .1
            .contains(PteFlags::R | PteFlags::W | PteFlags::V)
    );

    first
        .protect(
            &first_map,
            VirtAddr::from_raw(BASE + PAGE_SIZE),
            PAGE_SIZE,
            PteFlags::R,
        )
        .expect("tree: partial protect");
    assert_eq!(first_map.ranges().expect("tree: ranges").len(), 2);
    let protected = first
        .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
        .unwrap()
        .1;
    assert!(protected.contains(PteFlags::R | PteFlags::V | PteFlags::A | PteFlags::D));
    assert!(!protected.contains(PteFlags::W));
    first
        .unmap(&first_map, VirtAddr::from_raw(BASE), PAGE_SIZE)
        .expect("tree: partial unmap");
    assert_eq!(
        first_map.initial_range(),
        (VirtAddr::from_raw(BASE), 2 * PAGE_SIZE)
    );
    assert_eq!(first_map.frame_count(), 1);
    assert!(first.find(VirtAddr::from_raw(BASE)).is_none());
    assert!(Arc::ptr_eq(
        &first.find(VirtAddr::from_raw(BASE + PAGE_SIZE)).unwrap(),
        &first_map,
    ));
    assert!(first.translate(VirtAddr::from_raw(BASE)).is_none());
    assert!(second.translate(VirtAddr::from_raw(BASE)).is_some());
    assert!(Arc::ptr_eq(
        &second.find(VirtAddr::from_raw(BASE)).unwrap(),
        &second_map
    ));

    first
        .unmap(&first_map, VirtAddr::from_raw(BASE + PAGE_SIZE), PAGE_SIZE)
        .expect("tree: final unmap");
    assert!(!first_map.is_live());
    assert_eq!(first.map_count(), 0);
    first.close().expect("tree: close");
    assert!(first_map.tree().is_none());

    let rekey = Tree::new().expect("tree: rekey root");
    let original = rekey
        .map_frames(
            VirtAddr::from_raw(BASE + 0xA0_000),
            frames.clone(),
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: original mapping");
    rekey
        .unmap(&original, VirtAddr::from_raw(BASE + 0xA0_000), PAGE_SIZE)
        .expect("tree: open first-page hole");
    let replacement = rekey
        .map_frames(
            VirtAddr::from_raw(BASE + 0xA0_000),
            vec![frames[0].clone()],
            PteFlags::R,
            PteFlags::R,
        )
        .expect("tree: reuse first-page hole");
    assert!(Arc::ptr_eq(
        &rekey.find(VirtAddr::from_raw(BASE + 0xA0_000)).unwrap(),
        &replacement
    ));
    assert!(Arc::ptr_eq(
        &rekey
            .find(VirtAddr::from_raw(BASE + 0xA0_000 + PAGE_SIZE))
            .unwrap(),
        &original
    ));
    rekey
        .unmap(&replacement, VirtAddr::from_raw(BASE + 0xA0_000), PAGE_SIZE)
        .expect("tree: remove replacement");
    assert!(rekey.find(VirtAddr::from_raw(BASE + 0xA0_000)).is_none());
    assert!(Arc::ptr_eq(
        &rekey
            .find(VirtAddr::from_raw(BASE + 0xA0_000 + PAGE_SIZE))
            .unwrap(),
        &original
    ));
    let middle_map = rekey
        .map_frames(
            VirtAddr::from_raw(BASE + 0xB0_000),
            vec![frames[0].clone(), frames[1].clone(), frames[0].clone()],
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: three-page original");
    rekey
        .unmap(
            &middle_map,
            VirtAddr::from_raw(BASE + 0xB0_000 + PAGE_SIZE),
            PAGE_SIZE,
        )
        .expect("tree: open middle hole");
    let middle_replacement = rekey
        .map_frames(
            VirtAddr::from_raw(BASE + 0xB0_000 + PAGE_SIZE),
            vec![frames[1].clone()],
            PteFlags::R,
            PteFlags::R,
        )
        .expect("tree: reuse middle hole");
    rekey
        .unmap(
            &middle_replacement,
            VirtAddr::from_raw(BASE + 0xB0_000 + PAGE_SIZE),
            PAGE_SIZE,
        )
        .expect("tree: remove middle replacement");
    assert!(
        rekey
            .find(VirtAddr::from_raw(BASE + 0xB0_000 + PAGE_SIZE))
            .is_none()
    );
    assert!(Arc::ptr_eq(
        &rekey
            .find(VirtAddr::from_raw(BASE + 0xB0_000 + 2 * PAGE_SIZE))
            .unwrap(),
        &middle_map
    ));
    rekey.audit();
    rekey.close().expect("tree: close rekey root");
    second.close().expect("tree: close second");

    let source = Tree::new().expect("tree: source root");
    let source_map = source
        .map_frames(
            VirtAddr::from_raw(BASE + 0x80_000),
            frames.clone(),
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: source map");
    source
        .unmap(&source_map, VirtAddr::from_raw(BASE + 0x80_000), PAGE_SIZE)
        .expect("tree: source hole");
    let derived = Tree::new().expect("tree: derived root");
    assert_eq!(
        source_map
            .frames_in(VirtAddr::from_raw(BASE + 0x80_000 + PAGE_SIZE), PAGE_SIZE)
            .unwrap()
            .len(),
        1
    );
    let derived_map = derived
        .derive_map(
            &source_map,
            PAGE_SIZE,
            PAGE_SIZE,
            VirtAddr::from_raw(BASE + 0x20_000),
            PteFlags::R | PteFlags::X,
            PteFlags::R | PteFlags::X,
        )
        .expect("tree: derive retained fragment");
    assert_eq!(
        derived
            .translate(VirtAddr::from_raw(BASE + 0x20_000 + 37))
            .unwrap()
            .0,
        crate::memory::manager::addr::PhysAddr::from_raw(frames[1].physical_address() + 37),
    );
    assert_eq!(derived_map.frame_count(), 1);

    let lazy = derived
        .map_lazy(
            VirtAddr::from_raw(BASE + 0x40_000),
            2 * PAGE_SIZE,
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: lazy map");
    assert!(
        derived
            .translate(VirtAddr::from_raw(BASE + 0x40_000))
            .is_none()
    );
    let lazy_derived = Tree::new().expect("tree: lazy derived root");
    let lazy_map = lazy_derived
        .derive_map(
            &lazy,
            0,
            2 * PAGE_SIZE,
            VirtAddr::from_raw(BASE + 0x60_000),
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: derive untouched lazy range");
    let source_frames = lazy
        .frames_in(VirtAddr::from_raw(BASE + 0x40_000), 2 * PAGE_SIZE)
        .unwrap();
    let target_frames = lazy_map
        .frames_in(VirtAddr::from_raw(BASE + 0x60_000), 2 * PAGE_SIZE)
        .unwrap();
    assert_eq!(source_frames.len(), 2);
    assert!(
        source_frames
            .iter()
            .zip(&target_frames)
            .all(|((_, a), (_, b))| Arc::ptr_eq(a, b))
    );
    derived
        .materialize(&lazy, VirtAddr::from_raw(BASE + 0x40_000), PAGE_SIZE)
        .expect("tree: materialize one page");
    assert!(
        derived
            .translate(VirtAddr::from_raw(BASE + 0x40_000))
            .is_some()
    );
    assert!(
        derived
            .translate(VirtAddr::from_raw(BASE + 0x41_000))
            .is_some()
    );

    let narrow_lazy = derived
        .map_lazy(
            VirtAddr::from_raw(BASE + 0x70_000),
            2 * PAGE_SIZE,
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: lazy narrow map");
    derived
        .narrow(
            &narrow_lazy,
            VirtAddr::from_raw(BASE + 0x70_000),
            2 * PAGE_SIZE,
            PteFlags::R,
        )
        .expect("tree: narrow current and ceiling");
    derived
        .materialize(
            &narrow_lazy,
            VirtAddr::from_raw(BASE + 0x70_000),
            2 * PAGE_SIZE,
        )
        .expect("tree: materialize narrowed lazy pages");
    assert!(
        derived
            .translate(VirtAddr::from_raw(BASE + 0x70_000))
            .unwrap()
            .1
            .contains(PteFlags::R)
    );
    assert!(
        !derived
            .translate(VirtAddr::from_raw(BASE + 0x70_000))
            .unwrap()
            .1
            .contains(PteFlags::W)
    );

    let protect_lazy = derived
        .map_lazy(
            VirtAddr::from_raw(BASE + 0x90_000),
            2 * PAGE_SIZE,
            PteFlags::R | PteFlags::W | PteFlags::X,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: lazy protect map");
    derived
        .protect(
            &protect_lazy,
            VirtAddr::from_raw(BASE + 0x90_000),
            2 * PAGE_SIZE,
            PteFlags::R | PteFlags::X,
        )
        .expect("tree: protect unmaterialized range");
    derived
        .materialize(
            &protect_lazy,
            VirtAddr::from_raw(BASE + 0x90_000),
            2 * PAGE_SIZE,
        )
        .expect("tree: materialize protected lazy pages");
    let protected_flags = derived
        .translate(VirtAddr::from_raw(BASE + 0x90_000))
        .unwrap()
        .1;
    assert!(protected_flags.contains(PteFlags::R | PteFlags::X));
    assert!(!protected_flags.contains(PteFlags::W));

    let borrowed = unsafe {
        BorrowedRegion::new(
            crate::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address()),
            PAGE_SIZE,
        )
    }
    .expect("tree: reserve external page");
    let borrowed_map = derived
        .map_borrowed(
            VirtAddr::from_raw(BASE + 0x60_000),
            borrowed.clone(),
            0,
            PAGE_SIZE,
            PteFlags::R,
            PteFlags::R,
        )
        .expect("tree: borrow external page");
    assert_eq!(
        derived
            .translate(VirtAddr::from_raw(BASE + 0x60_000 + 15))
            .unwrap()
            .0,
        crate::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address() + 15),
    );
    derived
        .narrow(
            &borrowed_map,
            VirtAddr::from_raw(BASE + 0x60_000),
            PAGE_SIZE,
            PteFlags::R,
        )
        .expect("tree: preserve narrow ceiling");
    assert!(
        derived
            .protect(
                &borrowed_map,
                VirtAddr::from_raw(BASE + 0x60_000),
                PAGE_SIZE,
                PteFlags::W
            )
            .is_err()
    );
    assert!(derived.maps().unwrap().len() >= 3);
    let ranges = derived.ranges().unwrap();
    assert!(
        ranges
            .windows(2)
            .all(|pair| pair[0].1.start <= pair[1].1.start)
    );
    let large_start = VirtAddr::from_raw(BASE + 0x20_0000);
    let large_tree = Tree::new().expect("tree: large leaf root");
    let large_region = unsafe {
        BorrowedRegion::new(
            crate::memory::manager::addr::PhysAddr::from_raw(0x4000_0000),
            2 * 1024 * 1024,
        )
    }
    .expect("tree: reserve large test interval");
    let large_map = large_tree
        .map_borrowed(
            large_start,
            large_region,
            0,
            2 * 1024 * 1024,
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: map large borrowed leaf");
    assert_eq!(large_tree.leaf_span(large_start), Some(2 * 1024 * 1024));
    large_tree
        .protect(&large_map, large_start + PAGE_SIZE, PAGE_SIZE, PteFlags::R)
        .expect("tree: split large leaf for protect");
    assert_eq!(
        large_tree.leaf_span(large_start + PAGE_SIZE),
        Some(PAGE_SIZE)
    );
    large_tree.close().expect("tree: close large leaf root");

    let top = VirtAddr::from_raw(usize::MAX - (PAGE_SIZE - 1));
    let top_tree = Tree::new().expect("tree: top root");
    let top_map = top_tree
        .map_frames(
            top,
            vec![frames[0].clone()],
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: map final address page");
    assert_eq!(top_map.initial_range(), (top, PAGE_SIZE));
    top_tree
        .protect(&top_map, top, PAGE_SIZE, PteFlags::R)
        .expect("tree: protect final page");
    let top_derived = Tree::new().expect("tree: top derived root");
    top_derived
        .derive_map(&top_map, 0, PAGE_SIZE, top, PteFlags::R, PteFlags::R)
        .expect("tree: derive final page");
    top_derived.close().expect("tree: close top derived");
    top_tree
        .unmap(&top_map, top, PAGE_SIZE)
        .expect("tree: unmap final page");
    top_tree.close().expect("tree: close top root");

    source.audit();
    derived.audit();
    derived.close().expect("tree: close derived root");
    source.close().expect("tree: close source root");
}
