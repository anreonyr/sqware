#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::{Access, NestError, View, ViewRange, identity};
use crate::core::work::grant::Grant;
use crate::core::work::unit::space::SpaceBuilder;

use super::support;

/// 一段页对齐窗口在某个关系上的身份选择。
fn select(view: &Arc<View>, start: VirtAddr, size: usize) -> ViewRange {
    let offset = (start.as_usize() - view.initial_start().as_usize()) / PAGE_SIZE;
    view.range(
        PageOffset::new(offset),
        PageCount::new(size / PAGE_SIZE).expect("frame: page count"),
    )
    .expect("frame: selection")
}

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
    let flags = Access::READ;
    let first = support::map_frames(
        &a,
        VirtAddr::wrap(BASE),
        frames.clone(),
        Access::READ,
        flags,
    )
    .unwrap();
    let second =
        support::map_frames(&b, VirtAddr::wrap(BASE), frames, Access::READ, flags).unwrap();
    for (index, physical) in addresses.iter().enumerate() {
        let at = VirtAddr::wrap(BASE + index * PAGE_SIZE);
        assert_eq!(a.nest.translate(at).unwrap().0.as_usize(), *physical);
        assert_eq!(b.nest.translate(at).unwrap().0.as_usize(), *physical);
    }
    assert_eq!(
        a.nest.set_access(
            select(&first, VirtAddr::wrap(BASE), 3 * PAGE_SIZE),
            Access::READ | Access::WRITE
        ),
        Err(NestError::WidenDenied)
    );
    a.nest
        .detach(select(&first, VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE))
        .unwrap();
    assert!(a.nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_none());
    assert!(b.nest.translate(VirtAddr::wrap(BASE + PAGE_SIZE)).is_some());
    assert_eq!(
        a.nest
            .translate(VirtAddr::wrap(BASE + 2 * PAGE_SIZE))
            .unwrap()
            .0
            .as_usize(),
        addresses[2]
    );
    assert_eq!(
        a.nest.set_access(
            select(&first, VirtAddr::wrap(BASE + 2 * PAGE_SIZE), PAGE_SIZE),
            Access::READ | Access::WRITE
        ),
        Err(NestError::WidenDenied)
    );
    a.nest
        .set_access(
            select(&first, VirtAddr::wrap(BASE + 2 * PAGE_SIZE), PAGE_SIZE),
            Access::READ,
        )
        .unwrap();
    b.nest
        .detach(select(&second, VirtAddr::wrap(BASE), PAGE_SIZE))
        .unwrap();
    assert!(Arc::ptr_eq(
        &a.nest.probe(VirtAddr::wrap(BASE)).unwrap(),
        &first
    ));
    a.nest.audit();
    b.nest.audit();
    drop(first);
    drop(second);
    drop(a);
    // 来源级保留：撤销 PTE 不释放页；a 与 b 各自的 Flat 来源都强持这三帧。
    assert!(lifetimes.iter().all(|frame| frame.upgrade().is_some()));
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
        support::map_frames(
            space,
            VirtAddr::wrap(BASE),
            vec![frame],
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
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
    let mapping = support::map_frames(
        &space,
        VirtAddr::wrap(BASE),
        vec![frame],
        Access::READ | Access::WRITE,
        Access::READ | Access::WRITE,
    )
    .unwrap();
    let stale = support::view_at(&space, VirtAddr::wrap(BASE), PAGE_SIZE).unwrap();
    identity::fail_next_shootdown(space.nest.asid());
    assert_eq!(
        space.nest.detach(mapping.whole()),
        Err(NestError::Shootdown)
    );
    assert!(weak.upgrade().is_some());
    assert!(support::reserve(&space, VirtAddr::wrap(BASE), PAGE_SIZE).is_err());
    space.nest.detach(mapping.whole()).unwrap();
    // 撤销 PTE 不释放页：该 Frame 仍由关系的 Flat 来源强持。
    assert!(weak.upgrade().is_some());
    let replacement = support::map_lazy(
        &space,
        support::reserve(&space, VirtAddr::wrap(BASE), PAGE_SIZE).unwrap(),
        Access::READ,
        Access::READ,
    )
    .unwrap();
    space.nest.detach(stale.whole()).unwrap();
    assert!(Arc::ptr_eq(
        &space.nest.probe(VirtAddr::wrap(BASE)).unwrap(),
        &replacement
    ));
    space.nest.detach(replacement.whole()).unwrap();

    let reservation = support::reserve(&space, VirtAddr::wrap(BASE), PAGE_SIZE).unwrap();
    let frame = Frame::allocate().unwrap();
    let failed = Arc::downgrade(&frame);
    crate::core::memory::nest::fail_cleanup_prepare();
    assert!(matches!(
        support::attach_frames(&space, reservation, vec![frame], Access::READ, Access::READ),
        Err(NestError::OutOfMemory)
    ));
    assert!(failed.upgrade().is_none());
    let reservation = support::reserve(&space, VirtAddr::wrap(BASE), PAGE_SIZE).unwrap();
    space.nest.release_reservation(reservation).unwrap();
    let entire = support::reserve(&space, VirtAddr::wrap(BASE), 3 * PAGE_SIZE).unwrap();
    let (prefix, tail) = space.nest.split_reservation(entire, PAGE_SIZE).unwrap();
    let (middle, suffix) = space.nest.split_reservation(tail, PAGE_SIZE).unwrap();
    space.nest.release_reservation(prefix).unwrap();
    let reused = support::reserve(&space, VirtAddr::wrap(BASE), PAGE_SIZE).unwrap();
    assert!(support::reserve(&space, VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE).is_err());
    space.nest.release_reservation(middle).unwrap();
    let middle = support::reserve(&space, VirtAddr::wrap(BASE + PAGE_SIZE), PAGE_SIZE).unwrap();
    space.nest.release_reservation(reused).unwrap();
    space.nest.release_reservation(middle).unwrap();
    space.nest.release_reservation(suffix).unwrap();

    // 来源级保留：最后一份关系 / 来源持有消失后才回收该页。
    assert!(weak.upgrade().is_some());
    drop(mapping);
    drop(replacement);
    drop(stale);
    drop(space);
    assert!(weak.upgrade().is_none());
}
