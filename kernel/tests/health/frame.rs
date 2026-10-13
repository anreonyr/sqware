#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::{Frame, MapError, addr::VirtAddr, asid, entry::PteFlags};
use crate::core::memory::manager::RegionId;
use crate::core::work::grant::Grant;
use crate::core::work::unit::space::SpaceBuilder;

pub fn sharing() {
    const BASE: usize = 0x4000_0000;
    let a = SpaceBuilder::user().build().unwrap();
    let b = SpaceBuilder::user().build().unwrap();
    let mut frames = Vec::new();
    for _ in 0..3 {
        frames.push(Frame::allocate().unwrap());
    }
    let addresses: Vec<_> = frames
        .iter()
        .map(|frame| frame.physical_address())
        .collect();
    let lifetimes: Vec<_> = frames.iter().map(Arc::downgrade).collect();
    let flags = PteFlags::R | PteFlags::A | PteFlags::D;
    let first = a
        .memory
        .map_frames(
            a.memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), 3 * PAGE_SIZE)
                .unwrap(),
            frames.clone(),
            PteFlags::R,
            flags,
        )
        .unwrap();
    let second = b
        .memory
        .map_frames(
            b.memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), 3 * PAGE_SIZE)
                .unwrap(),
            frames,
            PteFlags::R,
            flags,
        )
        .unwrap();
    for (index, physical) in addresses.iter().enumerate() {
        let at = VirtAddr::wrap(BASE + index * PAGE_SIZE);
        assert_eq!(a.memory.backend().translate(at).unwrap().0.as_usize(), *physical);
        assert_eq!(b.memory.backend().translate(at).unwrap().0.as_usize(), *physical);
    }
    assert_eq!(
        a.memory.backend().protect(
            &first.map,
            VirtAddr::wrap(BASE),
            3 * PAGE_SIZE,
            PteFlags::R | PteFlags::W
        ),
        Err(MapError::WidenDenied)
    );
    a.memory
        .backend()
        .unmap(&first.map, VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE)
        .unwrap();
    assert!(a.memory.backend().translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    assert!(b.memory.backend().translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_some());
    assert_eq!(
        a.memory
            .backend()
            .translate(VirtAddr::wrap(BASE + 2 * PAGE_SIZE))
            .unwrap()
            .0
            .as_usize(),
        addresses[2]
    );
    assert_eq!(
        a.memory.backend().protect(
            &first.map,
            VirtAddr::wrap(BASE + 2 * PAGE_SIZE),
            PAGE_SIZE,
            PteFlags::R | PteFlags::W
        ),
        Err(MapError::WidenDenied)
    );
    a.memory
        .backend()
        .protect(
            &first.map,
            VirtAddr::wrap(BASE + 2 * PAGE_SIZE),
            PAGE_SIZE,
            PteFlags::R,
        )
        .unwrap();
    b.memory
        .backend()
        .unmap(&second.map, VirtAddr::wrap(BASE), PAGE_SIZE)
        .unwrap();
    assert!(Arc::ptr_eq(
        &a.memory.backend().find(VirtAddr::wrap(BASE)).unwrap(),
        &first.map
    ));
    a.memory.backend().audit();
    b.memory.backend().audit();
    drop(a);
    assert!(lifetimes[0].upgrade().is_none());
    assert!(lifetimes[1].upgrade().is_some());
    assert!(lifetimes[2].upgrade().is_some());
    drop(b);
    assert!(lifetimes.iter().all(|frame| frame.upgrade().is_none()));

    let a = SpaceBuilder::user().build().unwrap();
    let b = SpaceBuilder::user().build().unwrap();
    let mut addresses = [0; 2];
    let mut lifetimes = Vec::new();
    for (index, space) in [&a, &b].into_iter().enumerate() {
        let frame = Frame::allocate().unwrap();
        addresses[index] = frame.physical_address();
        lifetimes.push(Arc::downgrade(&frame));
        space
            .memory
            .map_frames(
                space
                    .memory
                    .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
                    .unwrap(),
                vec![frame],
                PteFlags::R | PteFlags::W,
                PteFlags::R | PteFlags::W,
            )
            .unwrap();
    }
    assert_ne!(addresses[0], addresses[1]);
    unsafe {
        *(addresses[0] as *mut u8) = 0x5a;
        assert_eq!(*(addresses[0] as *const u8), 0x5a);
        assert_eq!(*(addresses[1] as *const u8), 0);
    }
    drop(a);
    assert!(lifetimes[0].upgrade().is_none());
    assert!(lifetimes[1].upgrade().is_some());
    drop(b);
    assert!(lifetimes[1].upgrade().is_none());
}

pub fn authority() {
    use env::Permission;
    let root = Grant::try_new(Permission::FETCH | Permission::STORE | Permission::VEST).unwrap();
    let snapshot = root.clone();
    let child = Grant::try_new(Permission::FETCH | Permission::STORE).unwrap();
    root.narrow(Permission::FETCH | Permission::VEST).unwrap();
    assert_eq!(
        snapshot.permission(),
        Some(Permission::FETCH | Permission::VEST)
    );
    assert!(child.permission().unwrap().contains(Permission::STORE));
    child.invalidate();
    assert_eq!(child.permission(), None);
    assert_eq!(
        root.permission(),
        Some(Permission::FETCH | Permission::VEST)
    );
    root.invalidate();
    assert_eq!(snapshot.permission(), None);
    assert!(root.narrow(Permission::FETCH).is_err());
}

pub fn retirement() {
    const BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().unwrap();
    let frame = Frame::allocate().unwrap();
    let weak = Arc::downgrade(&frame);
    let mapping = space
        .memory
        .map_frames(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
                .unwrap(),
            vec![frame],
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W,
        )
        .unwrap();
    let stale = space.memory.mapping_at(VirtAddr::wrap(BASE), PAGE_SIZE).unwrap();
    asid::fail_next_shootdown(space.memory.backend().asid());
    assert_eq!(space.memory.unmap(&mapping), Err(MapError::Shootdown));
    assert!(weak.upgrade().is_some());
    assert!(
        space
            .memory
            .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
            .is_err()
    );
    space.memory.unmap(&mapping).unwrap();
    assert!(weak.upgrade().is_none());
    let replacement = space
        .memory
        .map_lazy(
            space
                .memory
                .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
                .unwrap(),
            PteFlags::R,
            PteFlags::R,
        )
        .unwrap();
    space.memory.unmap(&stale).unwrap();
    assert!(Arc::ptr_eq(
        &space.memory.backend().find(VirtAddr::wrap(BASE)).unwrap(),
        &replacement.map
    ));
    space.memory.unmap(&replacement).unwrap();

    let reservation = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
        .unwrap();
    let frame = Frame::allocate().unwrap();
    let failed = Arc::downgrade(&frame);
    crate::core::memory::manager::memory::fail_cleanup_prepare();
    assert!(matches!(
        space.memory.map_frames(reservation, vec![frame], PteFlags::R, PteFlags::R),
        Err(MapError::OutOfMemory)
    ));
    assert!(failed.upgrade().is_none());
    let reservation = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
        .unwrap();
    space.memory.release_reservation(reservation).unwrap();
    let entire = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(BASE), 3 * PAGE_SIZE)
        .unwrap();
    let (prefix, tail) = space.memory.split_reservation(entire, PAGE_SIZE).unwrap();
    let (middle, suffix) = space.memory.split_reservation(tail, PAGE_SIZE).unwrap();
    space.memory.release_reservation(prefix).unwrap();
    let reused = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::wrap(BASE), PAGE_SIZE)
        .unwrap();
    assert!(
        space
            .memory
            .reserve(
                RegionId::Normal,
                VirtAddr::wrap(BASE + PAGE_SIZE),
                PAGE_SIZE
            )
            .is_err()
    );
    space.memory.release_reservation(middle).unwrap();
    let middle = space
        .memory
        .reserve(
            RegionId::Normal,
            VirtAddr::wrap(BASE + PAGE_SIZE),
            PAGE_SIZE,
        )
        .unwrap();
    space.memory.release_reservation(reused).unwrap();
    space.memory.release_reservation(middle).unwrap();
    space.memory.release_reservation(suffix).unwrap();
}
