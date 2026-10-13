use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::Ordering;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::TABLE_LIVE;
use crate::memory::manager::{Frame, MapError, Tree, asid};

pub fn invalidation() {
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let tree = Tree::new().unwrap();
    let va = VirtAddr::from_raw(0x6200_0000);
    let frame = Frame::allocate().unwrap();
    let weak = Arc::downgrade(&frame);
    asid::fail_next_shootdown(tree.asid());
    assert_eq!(
        tree.map_frames(
            va,
            vec![frame],
            PteFlags::R | PteFlags::W,
            PteFlags::R | PteFlags::W
        )
        .err()
        .unwrap(),
        MapError::Shootdown,
    );
    assert!(tree.has_pending_invalidation());
    assert!(tree.find(va).is_none());
    assert!(tree.translate(va).is_none());
    assert!(weak.upgrade().is_some());
    let pending = tree.pending_maps().unwrap();
    assert_eq!(pending.len(), 1);
    let map = pending[0].clone();
    drop(pending);
    tree.retry_invalidation().unwrap();
    assert!(Arc::ptr_eq(&tree.find(va).unwrap(), &map));
    let installed_tables = TABLE_LIVE.load(Ordering::Relaxed);

    asid::fail_next_shootdown(tree.asid());
    assert_eq!(
        tree.unmap(&map, va, PAGE_SIZE).err().unwrap(),
        MapError::Shootdown
    );
    assert!(weak.upgrade().is_some());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed_tables);
    assert_eq!(
        tree.map_lazy(va, PAGE_SIZE, PteFlags::R, PteFlags::R)
            .err()
            .unwrap(),
        MapError::Shootdown,
    );
    tree.retry_invalidation().unwrap();
    assert!(weak.upgrade().is_none());
    assert!(TABLE_LIVE.load(Ordering::Relaxed) < installed_tables);
    let reused = tree
        .map_lazy(va, PAGE_SIZE, PteFlags::R, PteFlags::R)
        .unwrap();
    tree.unmap(&reused, va, PAGE_SIZE).unwrap();

    let frame = Frame::allocate().unwrap();
    let weak = Arc::downgrade(&frame);
    let old = tree
        .map_frames(va, vec![frame], PteFlags::R, PteFlags::R)
        .unwrap();
    let old_asid = tree.asid();
    let installed_tables = TABLE_LIVE.load(Ordering::Relaxed);
    asid::fail_next_shootdown(old_asid);
    assert_eq!(tree.close().err().unwrap(), MapError::Shootdown);
    assert!(tree.has_pending_invalidation());
    assert!(weak.upgrade().is_some());
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), installed_tables);
    tree.retry_invalidation().unwrap();
    assert!(weak.upgrade().is_none());
    assert!(old.tree().is_none());
    drop(tree);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);

    let replacement = Tree::new().unwrap();
    assert_eq!(replacement.asid(), old_asid);
    let current = replacement
        .map_lazy(va, PAGE_SIZE, PteFlags::R, PteFlags::R)
        .unwrap();
    assert_eq!(
        replacement.protect(&old, va, PAGE_SIZE, PteFlags::R),
        Err(MapError::NoRegion)
    );
    assert!(Arc::ptr_eq(&replacement.find(va).unwrap(), &current));
    replacement.close().unwrap();
    drop(replacement);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}
