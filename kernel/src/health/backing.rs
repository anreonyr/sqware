#![cfg(debug_assertions)]

use alloc::sync::Arc;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::{MapError, addr::VirtAddr, entry::PteFlags};
use crate::work::unit::space::{Backing, SpaceBuilder};

pub fn sharing() {
    const BASE: usize = 0x4000_0000;
    let a = SpaceBuilder::user().build().unwrap();
    let b = SpaceBuilder::user().build().unwrap();
    let backing = Backing::allocate(3 * PAGE_SIZE).unwrap();
    assert!(backing.owned());
    assert_eq!(backing.size(), 3 * PAGE_SIZE);
    let weak = Arc::downgrade(&backing);
    let first = backing.address(0, 3 * PAGE_SIZE).unwrap();
    let flags = a.pte_policy(PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D);
    for space in [&a, &b] {
        space.with_flush(|inner| inner.backed(
            VirtAddr::wrap(BASE), backing.clone(), 0, 3 * PAGE_SIZE, flags, PteFlags::R,
        )).unwrap();
    }
    drop(backing);
    assert_eq!(a.translate(VirtAddr::wrap(BASE)).unwrap().0, first);
    assert_eq!(b.translate(VirtAddr::wrap(BASE)).unwrap().0, first);
    assert!(matches!(a.protect_user(BASE, 3 * PAGE_SIZE, 6), Err(MapError::WidenDenied)));
    a.unmap(VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE).unwrap();
    assert!(a.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    assert_eq!(a.translate(VirtAddr::wrap(BASE + 2 * PAGE_SIZE)).unwrap().0, first + 2 * PAGE_SIZE);
    assert!(matches!(a.protect_user(BASE + 2 * PAGE_SIZE, PAGE_SIZE, 6), Err(MapError::WidenDenied)));
    a.protect_user(BASE + 2 * PAGE_SIZE, PAGE_SIZE, 2).unwrap();
    b.unmap(VirtAddr::wrap(BASE), PAGE_SIZE).unwrap();
    assert_eq!(b.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).unwrap().0, first + PAGE_SIZE);
    a.audit();
    b.audit();
    drop(a);
    assert!(weak.upgrade().is_some());
    drop(b);
    assert!(weak.upgrade().is_none());

    let a = SpaceBuilder::user().build().unwrap();
    let b = SpaceBuilder::user().build().unwrap();
    let mut addresses = [0; 2];
    let mut lifetimes = alloc::vec::Vec::new();
    for (index, space) in [&a, &b].into_iter().enumerate() {
        let backing = Backing::allocate(PAGE_SIZE).unwrap();
        addresses[index] = backing.address(0, PAGE_SIZE).unwrap().as_usize();
        lifetimes.push(Arc::downgrade(&backing));
        space.with_flush(|inner| {
            inner.backed(VirtAddr::wrap(BASE), backing, 0, PAGE_SIZE,
                space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W), PteFlags::R | PteFlags::W)?;
            inner.private(VirtAddr::wrap(BASE));
            Ok::<_, MapError>(())
        }).unwrap();
    }
    assert_ne!(addresses[0], addresses[1]);
    assert_eq!(a.translate(VirtAddr::wrap(BASE)).unwrap().0.as_usize(), addresses[0]);
    assert_eq!(b.translate(VirtAddr::wrap(BASE)).unwrap().0.as_usize(), addresses[1]);
    // SAFETY: both live Spaces retain separate writable backing allocations.
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
    let backing = Backing::allocate(PAGE_SIZE).unwrap();
    let root = backing.permit(Permission::FETCH | Permission::STORE | Permission::VEST);
    let snapshot = root.clone();
    assert!(backing.exclusive());
    let child = backing.permit(Permission::FETCH | Permission::STORE);
    root.narrow(Permission::FETCH | Permission::VEST);
    assert_eq!(snapshot.permission(), Permission::FETCH | Permission::VEST);
    assert!(!backing.readonly());
    assert!(!backing.exclusive());
    child.invalidate();
    assert!(backing.readonly());
    assert!(backing.exclusive());
    root.invalidate();
    assert!(snapshot.permission().is_empty());
    assert!(backing.readonly());
    assert!(!backing.exclusive());
}

pub fn retirement() {
    use crate::work::unit::space::Salvage;
    const BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().unwrap();
    let backing = Backing::allocate(PAGE_SIZE).unwrap();
    space.with_flush(|inner| inner.backed(VirtAddr::wrap(BASE), backing.clone(), 0, PAGE_SIZE,
        space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W), PteFlags::R | PteFlags::W)).unwrap();
    let mut salvage = Salvage::new();
    space.with_flush(|inner| inner.unmap(VirtAddr::wrap(BASE), PAGE_SIZE, &mut salvage)).unwrap();
    assert!(space.translate(VirtAddr::wrap(BASE)).is_none());
    assert!(!backing.unmapped());
    assert!(!backing.readonly());
    salvage.reclaim(&space).unwrap();
    assert!(backing.unmapped());
    assert!(backing.readonly());
    space.with(|inner| {
        inner.dynamic(BASE);
        let segment = inner.user.as_mut().unwrap();
        assert!(segment.allocate(BASE, 3 * PAGE_SIZE).is_ok());
        segment.prepare_cut().unwrap();
        let middle = segment.retire(BASE + PAGE_SIZE, PAGE_SIZE);
        assert!(!segment.allocate(BASE + PAGE_SIZE, PAGE_SIZE).is_ok());
        segment.prepare_cut().unwrap();
        let prefix = segment.retire(BASE, 2 * PAGE_SIZE);
        segment.reclaim(prefix);
        assert!(segment.allocate(BASE, PAGE_SIZE).is_ok());
        assert!(!segment.allocate(BASE + PAGE_SIZE, PAGE_SIZE).is_ok());
        segment.reclaim(middle);
        assert!(segment.allocate(BASE + PAGE_SIZE, PAGE_SIZE).is_ok());
        segment.reclaim(prefix);
        assert!(segment.holds(BASE, PAGE_SIZE));
        assert!(segment.holds(BASE + PAGE_SIZE, PAGE_SIZE));
    });
}
