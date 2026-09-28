#![cfg(debug_assertions)]

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::SpaceBuilder;

pub fn pagetable() {
    let levels = crate::memory::manager::mode::geometry(crate::memory::manager::mode::mode()).levels
        as usize;
    const BASE: usize = 0x4000_0000;
    const SIZE: usize = 4 * 1024 * 1024;
    const ROUNDS: usize = 32;

    let space = SpaceBuilder::user()
        .build()
        .expect("[health] pagetable: build space");
    let flags =
        space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
    let base_count = space.table_count();
    let held_before = crate::memory::allocator::statistics::frame_occupied()
        - crate::memory::allocator::statistics::block_occupied();

    for round in 0..ROUNDS {
        let mut frames: Vec<crate::memory::manager::table::Frame> = Vec::new();
        for _ in 0..(SIZE / PAGE_SIZE) {
            frames.push(crate::tag!(Probe, unsafe {
                Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                    .expect("[health] pagetable: data frame")
                    .assume_init()
            }));
        }
        let _pa = PhysAddr::from_raw(frames[0].as_ptr() as usize);
        space
            .attach(VirtAddr::from_raw(BASE), frames, flags)
            .expect("[health] pagetable: attach");
        crate::expect!(
            space.table_count() == base_count + levels,
            "round {round}: tables after map (got {} want {})",
            space.table_count(),
            base_count + levels
        );
        crate::expect!(
            space.translate(VirtAddr::from_raw(BASE)).is_some(),
            "round {round}: map hit"
        );

        space
            .unmap(VirtAddr::from_raw(BASE), SIZE)
            .expect("pagetable case: unmap");
        crate::expect!(
            space.table_count() == base_count,
            "round {round}: tables after unmap (got {} want {})",
            space.table_count(),
            base_count
        );
        crate::expect!(
            space.translate(VirtAddr::from_raw(BASE)).is_none(),
            "round {round}: unmap hit"
        );
        space.audit();
    }

    let held_after = crate::memory::allocator::statistics::frame_occupied()
        - crate::memory::allocator::statistics::block_occupied();
    crate::expect!(
        held_before == held_after,
        "net frames leaked: {held_before} → {held_after}（逐类 {}）",
        crate::memory::allocator::statistics::kinds()
    );
    drop(space);
}