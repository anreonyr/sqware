use alloc::sync::Arc;
use alloc::vec;

use ::core::sync::atomic::Ordering;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::Frame;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::table::TABLE_LIVE;
use crate::core::memory::manager::{BorrowedRegion, Tree};

pub fn manager_tree() {
    const BASE: usize = 0x6000_0000;
    let frames = vec![
        Frame::allocate().expect("tree: first frame"),
        Frame::allocate().expect("tree: second frame"),
    ];
    unsafe { ::core::ptr::write_volatile(frames[0].as_mut_ptr(), 0x5a) };
    let first = Tree::new().expect("tree: first root");
    assert_ne!(first.asid(), crate::core::memory::manager::asid::Asid::kernel());
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
        unsafe { ::core::ptr::read_volatile(frames[0].as_ptr()) },
        0x5a
    );
    assert!(Arc::ptr_eq(
        &first.find(VirtAddr::from_raw(BASE)).unwrap(),
        &first_map
    ));
    assert_eq!(
        first.translate(VirtAddr::from_raw(BASE + 37)).unwrap().0,
        crate::core::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address() + 37),
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
        crate::core::memory::manager::addr::PhysAddr::from_raw(frames[1].physical_address() + 37),
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
            crate::core::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address()),
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
        crate::core::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address() + 15),
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
            crate::core::memory::manager::addr::PhysAddr::from_raw(0x4000_0000),
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

    tree_hole_spanning_derive();
    tree_lazy_current_split();
    tree_frames_noncontiguous();
}

fn tree_hole_spanning_derive() {
    const BASE: usize = 0x7000_0000;
    const TARGET: usize = 0x7100_0000;
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = vec![
        Frame::allocate().expect("tree: hole source first"),
        Frame::allocate().expect("tree: hole source middle"),
        Frame::allocate().expect("tree: hole source third"),
    ];
    let source = Tree::new().expect("tree: hole source root");
    let source_map = source
        .map_frames(
            VirtAddr::from_raw(BASE),
            frames.clone(),
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: hole source map");
    let anchor = source_map.initial_range();
    assert_eq!(anchor, (VirtAddr::from_raw(BASE), 3 * PAGE_SIZE));
    source
        .unmap(&source_map, VirtAddr::from_raw(BASE + PAGE_SIZE), PAGE_SIZE)
        .expect("tree: open source middle hole");
    assert_eq!(source_map.initial_range(), anchor);
    assert_eq!(source_map.ranges().expect("tree: source ranges").len(), 2);
    let first_window = source_map
        .frames_in(VirtAddr::from_raw(BASE), PAGE_SIZE)
        .expect("tree: source first window");
    assert_eq!(first_window.len(), 1);
    assert!(Arc::ptr_eq(&first_window[0].1, &frames[0]));
    let third_window = source_map
        .frames_in(VirtAddr::from_raw(BASE + 2 * PAGE_SIZE), PAGE_SIZE)
        .expect("tree: source third window");
    assert_eq!(third_window.len(), 1);
    assert!(Arc::ptr_eq(&third_window[0].1, &frames[2]));
    assert!(source.translate(VirtAddr::from_raw(BASE + PAGE_SIZE)).is_none());

    let target = Tree::new().expect("tree: hole target root");
    let target_map = target
        .derive_map(
            &source_map,
            0,
            3 * PAGE_SIZE,
            VirtAddr::from_raw(TARGET),
            PteFlags::R | PteFlags::X,
            PteFlags::R | PteFlags::X,
        )
        .expect("tree: derive window across hole");
    assert_eq!(
        target_map.initial_range(),
        (VirtAddr::from_raw(TARGET), 3 * PAGE_SIZE)
    );
    let target_ranges = target_map.ranges().expect("tree: target ranges");
    assert_eq!(target_ranges.len(), 2);
    assert_eq!(target_ranges[0].start.as_usize(), TARGET);
    assert_eq!(target_ranges[1].start.as_usize(), TARGET + 2 * PAGE_SIZE);
    assert!(target.translate(VirtAddr::from_raw(TARGET + PAGE_SIZE)).is_none());
    assert_eq!(
        target
            .translate(VirtAddr::from_raw(TARGET))
            .expect("tree: target first window")
            .0,
        crate::core::memory::manager::addr::PhysAddr::from_raw(frames[0].physical_address()),
    );
    assert_eq!(
        target
            .translate(VirtAddr::from_raw(TARGET + 2 * PAGE_SIZE))
            .expect("tree: target third window")
            .0,
        crate::core::memory::manager::addr::PhysAddr::from_raw(frames[2].physical_address()),
    );
    let flags = target
        .translate(VirtAddr::from_raw(TARGET))
        .expect("tree: target first window")
        .1;
    assert!(flags.contains(PteFlags::R | PteFlags::X | PteFlags::V));
    assert!(!flags.contains(PteFlags::W));
    assert_eq!(target_map.frame_count(), 2);
    source.audit();
    target.audit();

    target
        .unmap(&target_map, VirtAddr::from_raw(TARGET), PAGE_SIZE)
        .expect("tree: unmap target first window");
    target
        .unmap(&target_map, VirtAddr::from_raw(TARGET + 2 * PAGE_SIZE), PAGE_SIZE)
        .expect("tree: unmap target third window");
    assert!(!target_map.is_live());
    target.close().expect("tree: close hole target root");
    source.close().expect("tree: close hole source root");
    drop(target_map);
    drop(target);
    drop(source_map);
    drop(source);
    drop(frames);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

fn tree_lazy_current_split() {
    const BASE: usize = 0x7200_0000;
    const TARGET: usize = 0x7300_0000;
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let tree = Tree::new().expect("tree: lazy root");
    let lazy = tree
        .map_lazy(
            VirtAddr::from_raw(BASE),
            2 * PAGE_SIZE,
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("tree: lazy two pages");
    assert!(tree.translate(VirtAddr::from_raw(BASE)).is_none());
    assert!(tree.translate(VirtAddr::from_raw(BASE + PAGE_SIZE)).is_none());
    tree.protect(
        &lazy,
        VirtAddr::from_raw(BASE + PAGE_SIZE),
        PAGE_SIZE,
        PteFlags::R,
    )
    .expect("tree: make second lazy page read-only");
    let split = lazy.ranges().expect("tree: lazy ranges");
    assert_eq!(split.len(), 2);
    assert_ne!(split[0].current.bits(), split[1].current.bits());
    assert!(split[0].current.contains(PteFlags::R | PteFlags::W));
    assert!(split[1].current.contains(PteFlags::R));
    assert!(!split[1].current.contains(PteFlags::W));

    tree.materialize(&lazy, VirtAddr::from_raw(BASE), 2 * PAGE_SIZE)
        .expect("tree: materialize both lazy pages");
    let writable = tree
        .translate(VirtAddr::from_raw(BASE))
        .expect("tree: first lazy page")
        .1;
    assert!(writable.contains(PteFlags::R | PteFlags::W | PteFlags::V));
    let readonly = tree
        .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
        .expect("tree: second lazy page")
        .1;
    assert!(readonly.contains(PteFlags::R | PteFlags::V));
    assert!(!readonly.contains(PteFlags::W));
    assert!(tree.check_access(VirtAddr::from_raw(BASE), PAGE_SIZE, PteFlags::W));
    assert!(!tree.check_access(VirtAddr::from_raw(BASE + PAGE_SIZE), PAGE_SIZE, PteFlags::W));
    assert!(tree.check_access(VirtAddr::from_raw(BASE + PAGE_SIZE), PAGE_SIZE, PteFlags::R));
    let zeros = [0u8; PAGE_SIZE];
    assert!(tree.copy_to(VirtAddr::from_raw(BASE), &zeros, PteFlags::W));
    assert!(!tree.copy_to(VirtAddr::from_raw(BASE + PAGE_SIZE), &zeros, PteFlags::W));
    let held = lazy
        .frames_in(VirtAddr::from_raw(BASE), 2 * PAGE_SIZE)
        .expect("tree: lazy frames");
    assert_eq!(held.len(), 2);
    assert!(held.iter().all(|(_, frame)| Arc::strong_count(frame) == 2));
    tree.audit();

    let derived_tree = Tree::new().expect("tree: lazy derived root");
    let derived = derived_tree
        .derive_map(
            &lazy,
            0,
            2 * PAGE_SIZE,
            VirtAddr::from_raw(TARGET),
            PteFlags::R | PteFlags::W,
            PteFlags::R,
        )
        .expect("tree: derive across differing source rights");
    let derived_ranges = derived.ranges().expect("tree: derived ranges");
    assert_eq!(derived_ranges.len(), 2);
    assert!(
        derived_ranges
            .iter()
            .all(|range| range.current.bits() == (PteFlags::R | PteFlags::V).bits())
    );
    assert_eq!(
        derived_tree
            .translate(VirtAddr::from_raw(TARGET))
            .expect("tree: derived first page")
            .0,
        crate::core::memory::manager::addr::PhysAddr::from_raw(held[0].1.physical_address()),
    );
    assert_eq!(
        derived_tree
            .translate(VirtAddr::from_raw(TARGET + PAGE_SIZE))
            .expect("tree: derived second page")
            .0,
        crate::core::memory::manager::addr::PhysAddr::from_raw(held[1].1.physical_address()),
    );
    assert!(!derived_tree.check_access(VirtAddr::from_raw(TARGET), PAGE_SIZE, PteFlags::W));
    assert!(derived_tree.check_access(VirtAddr::from_raw(TARGET), PAGE_SIZE, PteFlags::R));
    tree.audit();
    derived_tree.audit();
    assert!(held.iter().all(|(_, frame)| Arc::strong_count(frame) == 3));

    derived_tree
        .unmap(&derived, VirtAddr::from_raw(TARGET), 2 * PAGE_SIZE)
        .expect("tree: unmap derived window");
    assert!(!derived.is_live());
    derived_tree.close().expect("tree: close lazy derived root");
    tree.unmap(&lazy, VirtAddr::from_raw(BASE), 2 * PAGE_SIZE)
        .expect("tree: unmap lazy window");
    assert!(!lazy.is_live());
    tree.close().expect("tree: close lazy root");
    assert!(held.iter().all(|(_, frame)| Arc::strong_count(frame) == 1));
    drop(held);
    drop(derived);
    drop(derived_tree);
    drop(lazy);
    drop(tree);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

fn tree_frames_noncontiguous() {
    const BASE: usize = 0x7400_0000;
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let mut owned = vec![
        Frame::allocate().expect("tree: noncontiguous frame a"),
        Frame::allocate().expect("tree: noncontiguous frame b"),
        Frame::allocate().expect("tree: noncontiguous frame c"),
    ];
    owned.sort_by_key(|frame| frame.physical_address());
    let high = owned.pop().expect("tree: noncontiguous high frame");
    let low = owned.remove(0);
    let middle = owned.pop().expect("tree: noncontiguous middle frame");
    assert!(owned.is_empty());
    assert!(high.physical_address() - low.physical_address() >= 2 * PAGE_SIZE);
    let middle_physical = middle.physical_address();
    let middle_held = Arc::strong_count(&middle);
    assert_eq!(Arc::strong_count(&low), 1);
    assert_eq!(Arc::strong_count(&high), 1);

    let tree = Tree::new().expect("tree: noncontiguous root");
    let map = tree
        .map_frames(
            VirtAddr::from_raw(BASE),
            vec![low.clone(), high.clone()],
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .expect("tree: map two noncontiguous frames");
    assert_eq!(map.frame_count(), 2);
    let first = tree
        .translate(VirtAddr::from_raw(BASE))
        .expect("tree: noncontiguous first page")
        .0;
    let second = tree
        .translate(VirtAddr::from_raw(BASE + PAGE_SIZE))
        .expect("tree: noncontiguous second page")
        .0;
    assert_eq!(
        first,
        crate::core::memory::manager::addr::PhysAddr::from_raw(low.physical_address())
    );
    assert_eq!(
        second,
        crate::core::memory::manager::addr::PhysAddr::from_raw(high.physical_address())
    );
    assert_ne!(
        second,
        crate::core::memory::manager::addr::PhysAddr::from_raw(low.physical_address() + PAGE_SIZE)
    );
    let listed = map
        .frames_in(VirtAddr::from_raw(BASE), 2 * PAGE_SIZE)
        .expect("tree: noncontiguous frames");
    assert_eq!(listed.len(), 2);
    assert!(Arc::ptr_eq(&listed[0].1, &low));
    assert!(Arc::ptr_eq(&listed[1].1, &high));
    assert!(listed.iter().all(|(_, frame)| !Arc::ptr_eq(frame, &middle)));
    assert_eq!(Arc::strong_count(&low), 3);
    drop(listed);
    assert_eq!(Arc::strong_count(&low), 2);
    assert_eq!(Arc::strong_count(&high), 2);
    assert_eq!(Arc::strong_count(&middle), middle_held);
    assert_eq!(middle.physical_address(), middle_physical);
    tree.audit();

    tree.unmap(&map, VirtAddr::from_raw(BASE), 2 * PAGE_SIZE)
        .expect("tree: unmap noncontiguous pair");
    assert!(!map.is_live());
    assert_eq!(Arc::strong_count(&low), 1);
    assert_eq!(Arc::strong_count(&high), 1);
    assert_eq!(tree.map_count(), 0);
    tree.close().expect("tree: close noncontiguous root");
    drop(map);
    drop(tree);
    assert_eq!(Arc::strong_count(&low), 1);
    assert_eq!(Arc::strong_count(&high), 1);
    assert_eq!(Arc::strong_count(&middle), middle_held);
    drop(low);
    drop(high);
    drop(middle);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}
