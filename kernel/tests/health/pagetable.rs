#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::Frame;
use crate::core::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::RegionId;
use crate::core::work::unit::space::SpaceBuilder;

pub fn pagetable() {
    const BASE: usize = 0x4000_0000;
    const SIZE: usize = 4 * 1024 * 1024;
    const ROUNDS: usize = 32;

    let space = SpaceBuilder::user()
        .build()
        .expect("[health] pagetable: build space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    let base_count = space.memory.backend().table_count();
    let held_before = crate::core::memory::allocator::statistics::frame_occupied()
        - crate::core::memory::allocator::statistics::block_occupied();

    for round in 0..ROUNDS {
        let mut frames: Vec<Arc<Frame>> = Vec::new();
        for _ in 0..(SIZE / PAGE_SIZE) {
            frames.push(crate::tag!(
                Probe,
                Frame::allocate().expect("[health] pagetable: data frame")
            ));
        }
        let reservation = space
            .memory
            .reserve(RegionId::Normal, VirtAddr::from_raw(BASE), SIZE)
            .expect("[health] pagetable: reserve");
        let mapping = space
            .memory
            .map_frames(reservation, frames, flags, flags)
            .expect("[health] pagetable: map frames");
        assert_eq!(mapping.map.frame_count(), SIZE / PAGE_SIZE);
        crate::expect!(
            space.memory.backend().table_count() > base_count,
            "round {round}: map allocated no page-table nodes"
        );
        crate::expect!(
            space.memory.backend().translate(VirtAddr::from_raw(BASE)).is_some(),
            "round {round}: map hit"
        );

        space.memory.unmap(&mapping).expect("pagetable case: unmap");
        crate::expect!(
            space.memory.backend().table_count() == base_count,
            "round {round}: tables after unmap (got {} want {})",
            space.memory.backend().table_count(),
            base_count
        );
        crate::expect!(
            space.memory.backend().translate(VirtAddr::from_raw(BASE)).is_none(),
            "round {round}: unmap hit"
        );
        space.memory.backend().audit();
    }

    let held_after = crate::core::memory::allocator::statistics::frame_occupied()
        - crate::core::memory::allocator::statistics::block_occupied();
    crate::expect!(
        held_before == held_after,
        "net frames leaked: {held_before} → {held_after}（逐类 {}）",
        crate::core::memory::allocator::statistics::kinds()
    );
    drop(space);
}

pub fn kernel_layout() {
    use crate::machine::layout::{IMAGE_BASE, TRAMPOLINE};
    use crate::core::memory::manager::mode;
    use crate::core::memory::manager::RegionId;

    unsafe extern "C" {
        static _kernel_base: u8;
        static _text_end: u8;
        static _rodata_start: u8;
        static _kernel_edge: u8;
    }
    let text_start = (&raw const _kernel_base).addr();
    let text_end = (&raw const _text_end).addr();
    let rodata_start = (&raw const _rodata_start).addr();
    let image_end = (&raw const _kernel_edge).addr();
    for boundary in [text_start, text_end, rodata_start, image_end] {
        assert!(
            boundary.is_multiple_of(PAGE_SIZE),
            "kernel section boundary alignment"
        );
    }
    assert!(text_start < text_end && text_end <= rodata_start && rodata_start <= image_end);
    let ram = crate::machine::board::info().dram;
    let team = crate::core::work::unit::team::kernel().expect("kernel team");
    let space = &team.space;
    let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
    let rw = PteFlags::R | PteFlags::W;
    for pa in (ram.base..ram.base + ram.size).step_by(PAGE_SIZE) {
        let expected = if (text_start..text_end).contains(&pa) {
            PteFlags::R | PteFlags::X
        } else if (rodata_start..image_end).contains(&pa) {
            PteFlags::R
        } else {
            rw
        };
        for offset in [0, mode::lower().as_usize()] {
            let va = VirtAddr::wrap(offset + pa);
            let (physical, flags) = space.memory.backend().translate(va).expect("kernel RAM mapping");
            assert_eq!(physical, PhysAddr::from_raw(pa));
            assert_eq!(
                (flags & rwx).bits(),
                expected.bits(),
                "kernel mapping at {va:?}"
            );
            assert!(flags.contains(PteFlags::G) && !flags.contains(PteFlags::U));
        }
    }
    let (pa, flags) = space
        .memory
        .backend()
        .translate(TRAMPOLINE)
        .expect("trampoline mapping");
    assert_eq!(pa, crate::machine::layout::trampoline_pa());
    assert_eq!((flags & rwx).bits(), (PteFlags::R | PteFlags::X).bits());

    for hart in 0..crate::machine::hart::hart_count() {
        let id = crate::machine::hart::HartId::new(hart);
        let guard = crate::machine::layout::TRAP_STACK_BASE + hart * crate::machine::layout::TRAP_STACK_SLOT_SIZE;
        assert!(space.memory.backend().find(guard).is_none());
        assert!(space.memory.backend().translate(guard).is_none());
        let base = crate::machine::switcher::stack::trap_stack_base(id);
        let (physical, flags) = space.memory.backend().translate(base).expect("trap stack body mapping");
        let map = space.memory.backend().find(base).expect("trap stack body Map");
        assert_eq!(
            map.frames_in(base, PAGE_SIZE).expect("trap stack frame")[0]
                .1
                .physical_address(),
            physical.as_usize()
        );
        assert!(flags.contains(PteFlags::R | PteFlags::W));
        assert!(!flags.intersects(PteFlags::U | PteFlags::X));
        let boot_top = crate::machine::switcher::stack::boot_stack(hart);
        let boot_page = boot_top - PAGE_SIZE;
        assert!(boot_top.is_multiple_of(PAGE_SIZE));
        assert_eq!(
            space
                .memory
                .backend()
                .translate(VirtAddr::from_raw(boot_page))
                .expect("boot stack physical page")
                .0,
            PhysAddr::from_raw(boot_page)
        );
        let hart_frame = crate::machine::layout::HART_FRAME_BASE + hart * PAGE_SIZE;
        let (_, hart_flags) = space
            .memory
            .backend()
            .translate(hart_frame)
            .expect("hart frame mapping");
        assert!(hart_flags.contains(PteFlags::R | PteFlags::W));
        assert!(!hart_flags.intersects(PteFlags::U | PteFlags::X));
    }

    assert!(
        space
            .memory
            .reserve(RegionId::Normal, IMAGE_BASE, PAGE_SIZE)
            .is_err()
    );
    assert!(
        space
            .memory
            .backend()
            .translate(VirtAddr::from_raw(image_end))
            .is_some()
    );
    assert!(
        space
            .memory
            .reserve(RegionId::Normal, VirtAddr::from_raw(image_end), PAGE_SIZE)
            .is_err()
    );
    let reservation = space
        .memory
        .allocate_low(RegionId::Normal, PAGE_SIZE)
        .expect("skip reserved RAM interval");
    assert_eq!(reservation.start().as_usize(), ram.base + ram.size);
    space
        .memory
        .release_reservation(reservation)
        .expect("release allocation above RAM");
}

pub fn space_reservation_layout() {
    use crate::machine::layout::TASK_STACK_GUARD;
    use crate::core::memory::manager::mode;
    use crate::core::memory::manager::RegionId;

    const BASE: usize = 0x4000_0000;
    let edge = mode::upper().as_usize();
    let floor = BASE + 4 * PAGE_SIZE;
    let space = SpaceBuilder::user().build().expect("reservation space");
    space
        .memory
        .advance_region(RegionId::Normal, floor)
        .expect("image boundary");
    assert!(
        space
            .memory
            .reserve(RegionId::Normal, VirtAddr::from_raw(BASE), PAGE_SIZE)
            .is_err()
    );
    let tables = space.memory.backend().table_count();

    let heap_size = 2 * PAGE_SIZE;
    let heap_reservation = space
        .memory
        .allocate_low(RegionId::Normal, heap_size)
        .expect("heap above image");
    assert_eq!(heap_reservation.start().as_usize(), floor);
    let heap_frames = (0..heap_size / PAGE_SIZE)
        .map(|_| Frame::allocate().expect("heap frame"))
        .collect();
    let heap = space
        .memory
        .map_frames(
            heap_reservation,
            heap_frames,
            PteFlags::R | PteFlags::W,
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
        )
        .expect("heap mapping");

    let slot_size = TASK_STACK_GUARD + 2 * PAGE_SIZE;
    let claim_stack = |space: &crate::core::work::unit::space::Space| {
        let slot = space
            .memory
            .allocate_high(RegionId::Normal, slot_size)
            .expect("stack slot");
        let (guard, body) = space
            .memory
            .split_reservation(slot, TASK_STACK_GUARD)
            .expect("split stack guard");
        let frames = (0..2)
            .map(|_| Frame::allocate().expect("stack frame"))
            .collect();
        let mapping = space
            .memory
            .map_frames(
                body,
                frames,
                PteFlags::R | PteFlags::W,
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
            )
            .expect("stack mapping");
        (guard, mapping)
    };
    let (first_guard, first) = claim_stack(&space);
    let (second_guard, second) = claim_stack(&space);
    assert_eq!(
        first.reservation.start().as_usize() + first.reservation.size(),
        edge
    );
    assert_eq!(
        second.reservation.start() + second.reservation.size(),
        first_guard.start()
    );
    assert!(space.memory.backend().find(first_guard.start()).is_none());
    assert!(space.memory.backend().translate(first_guard.start()).is_none());
    for mapping in [&first, &second] {
        for offset in (0..mapping.reservation.size()).step_by(PAGE_SIZE) {
            let (_, flags) = space
                .memory
                .backend()
                .translate(mapping.reservation.start() + offset)
                .expect("stack body");
            assert!(flags.contains(PteFlags::U | PteFlags::R | PteFlags::W));
            assert!(!flags.contains(PteFlags::X));
        }
    }
    let first_va = first_guard.start();
    space.memory.unmap(&first).expect("release top stack");
    space
        .memory
        .release_reservation(first_guard)
        .expect("release first guard");
    let (reused_guard, reused) = claim_stack(&space);
    assert_eq!(reused_guard.start(), first_va);
    space.memory.unmap(&reused).expect("release reused stack");
    space
        .memory
        .release_reservation(reused_guard)
        .expect("release reused guard");
    space.memory.unmap(&second).expect("release second stack");
    space
        .memory
        .release_reservation(second_guard)
        .expect("release second guard");
    space.memory.unmap(&heap).expect("release heap");
    assert_eq!(space.memory.backend().table_count(), tables);

    let fixed = edge - 3 * PAGE_SIZE;
    let fixed_res = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::from_raw(fixed), 2 * PAGE_SIZE)
        .expect("fixed reservation");
    assert!(
        space
            .memory
            .reserve(
                RegionId::Normal,
                VirtAddr::from_raw(fixed + PAGE_SIZE),
                PAGE_SIZE
            )
            .is_err()
    );
    let top = space
        .memory
        .allocate_high(RegionId::Normal, PAGE_SIZE)
        .expect("top gap");
    assert_eq!(top.start().as_usize(), edge - PAGE_SIZE);
    let below = space
        .memory
        .allocate_high(RegionId::Normal, 2 * PAGE_SIZE)
        .expect("below fixed reservation");
    assert_eq!(below.start().as_usize(), fixed - 2 * PAGE_SIZE);
    space.memory.release_reservation(fixed_res).expect("release fixed");
    space.memory.release_reservation(top).expect("release top");
    space.memory.release_reservation(below).expect("release below");
    let full_size = edge - floor - PAGE_SIZE;
    let full = space
        .memory
        .reserve(RegionId::Normal, VirtAddr::from_raw(floor), full_size)
        .expect("fill lower gap");
    assert!(
        space
            .memory
            .allocate_high(RegionId::Normal, 2 * PAGE_SIZE)
            .is_err()
    );
    space.memory.release_reservation(full).expect("release full range");
    let last = space
        .memory
        .allocate_high(RegionId::Normal, PAGE_SIZE)
        .expect("last page after reclaim");
    assert_eq!(last.start().as_usize(), edge - PAGE_SIZE);
    space
        .memory
        .release_reservation(last)
        .expect("release reclaimed address");
    space.memory.backend().audit();
}

pub fn image_dynamic_region() {
    use crate::core::memory::manager::RegionId;

    #[repr(C, align(4096))]
    struct Image([u8; 2 * PAGE_SIZE]);
    const fn fixture() -> [u8; 2 * PAGE_SIZE] {
        const fn put(bytes: &mut [u8; 2 * PAGE_SIZE], at: usize, value: u64, len: usize) {
            let mut i = 0;
            while i < len {
                bytes[at + i] = (value >> (8 * i)) as u8;
                i += 1;
            }
        }
        let mut bytes = [0; 2 * PAGE_SIZE];
        let magic = env::ledger::capsule::MAGIC;
        let mut i = 0;
        while i < magic.len() {
            bytes[i] = magic[i];
            i += 1;
        }
        put(&mut bytes, 8, 1, 2);
        put(&mut bytes, 10, 12, 1);
        put(&mut bytes, 12, 2, 4);
        put(&mut bytes, 16, 0x10000, 8);
        put(&mut bytes, 24, (2 * PAGE_SIZE) as u64, 8);
        put(&mut bytes, 32, 0x10000, 8);
        put(&mut bytes, 40, 1, 8);
        put(&mut bytes, 48, 1, 8);
        put(&mut bytes, 56, PAGE_SIZE as u64, 8);
        put(&mut bytes, 64, 5, 4);
        put(&mut bytes, 72, 0x14000, 8);
        put(&mut bytes, 80, 4, 8);
        put(&mut bytes, 104, 3, 4);
        bytes
    }
    static IMAGE: Image = Image(fixture());
    let team = crate::core::work::unit::capsule::assemble(&IMAGE.0).expect("sparse capsule");
    let lazy = team
        .space
        .memory
        .backend()
        .find(VirtAddr::from_raw(0x17000))
        .expect("lazy BSS map");
    assert!(
        team.space
            .memory
            .backend()
            .translate(VirtAddr::from_raw(0x17000))
            .is_none()
    );
    let heap = team
        .space
        .memory
        .allocate_low(RegionId::Normal, PAGE_SIZE)
        .expect("heap after BSS");
    assert_eq!(heap.start().as_usize(), 0x18000);
    team.space
        .memory
        .release_reservation(heap)
        .expect("release image heap");
    team.space
        .memory
        .backend()
        .unmap(&lazy, VirtAddr::from_raw(0x17000), PAGE_SIZE)
        .expect("unmap lazy image page");
    let image = team
        .space
        .memory
        .backend()
        .find(VirtAddr::from_raw(0x10000))
        .expect("borrowed image map");
    team.space
        .memory
        .backend()
        .unmap(&image, VirtAddr::from_raw(0x10000), PAGE_SIZE)
        .expect("unmap borrowed image page");
    assert!(
        team.space
            .memory
            .reserve(RegionId::Normal, VirtAddr::from_raw(0x12000), PAGE_SIZE)
            .is_err()
    );
    let next = team
        .space
        .memory
        .allocate_low(RegionId::Normal, PAGE_SIZE)
        .expect("dynamic mapping after BSS");
    assert_eq!(next.start().as_usize(), 0x18000);
    team.space
        .memory
        .release_reservation(next)
        .expect("release dynamic range");
    team.space.memory.backend().audit();
}

pub fn interval_index() {
    const BASE: usize = 0x4000_0000;
    const COUNT: usize = 64;
    const STRIDE: usize = 16 * PAGE_SIZE;
    let space = SpaceBuilder::user().build().expect("interval space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    let tables = space.memory.backend().table_count();
    let mut maps = Vec::new();
    let mut replacements = Vec::new();
    for i in (0..COUNT).rev() {
        let va = VirtAddr::wrap(BASE + i * STRIDE);
        let reservation = space
            .memory
            .reserve(RegionId::Normal, va, 8 * PAGE_SIZE)
            .expect("reserve indexed range");
        maps.push(
            space
                .memory
                .map_lazy(reservation, flags, flags)
                .expect("indexed lazy region"),
        );
    }
    let ordered = space.memory.backend().ranges().expect("ordered interval ranges");
    assert!(
        ordered
            .windows(2)
            .all(|pair| pair[0].1.start < pair[1].1.start)
    );
    assert_eq!(space.memory.backend().map_count(), COUNT + 1);
    for i in 0..COUNT {
        let va = VirtAddr::wrap(BASE + i * STRIDE);
        let mapping = &maps[COUNT - i - 1];
        let read_only = PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::U;
        space
            .memory
            .backend()
            .protect(&mapping.map, va + 2 * PAGE_SIZE, 2 * PAGE_SIZE, read_only)
            .expect("split permissions");
        space
            .memory
            .backend()
            .materialize(&mapping.map, va + 2 * PAGE_SIZE, PAGE_SIZE)
            .expect("protected fault");
        let (_, pte) = space
            .memory
            .backend()
            .translate(va + 2 * PAGE_SIZE)
            .expect("protected page");
        assert!(pte.contains(PteFlags::R | PteFlags::U) && !pte.contains(PteFlags::W));
        space
            .memory
            .backend()
            .materialize(&mapping.map, va + 4 * PAGE_SIZE, PAGE_SIZE)
            .expect("writable fault");
        assert!(
            space
                .memory
                .backend()
                .translate(va + 4 * PAGE_SIZE)
                .expect("writable page")
                .1
                .contains(PteFlags::W)
        );
        space
            .memory
            .unmap_range(va, PAGE_SIZE)
            .expect("move mapping start");
        space
            .memory
            .unmap_range(va + 3 * PAGE_SIZE, 2 * PAGE_SIZE)
            .expect("cut across mapping");
        assert!(!space.memory.backend().overlaps(va, PAGE_SIZE).unwrap());
        assert!(space.memory.backend().overlaps(va + PAGE_SIZE, PAGE_SIZE).unwrap());
        assert!(
            !space
                .memory
                .backend()
                .overlaps(va + 3 * PAGE_SIZE, 2 * PAGE_SIZE)
                .unwrap()
        );
        assert!(Arc::ptr_eq(
            &mapping.map,
            &space
                .memory
                .backend()
                .find(va + PAGE_SIZE)
                .expect("left split retains Map")
        ));
        assert!(Arc::ptr_eq(
            &mapping.map,
            &space
                .memory
                .backend()
                .find(va + 5 * PAGE_SIZE)
                .expect("right split retains Map")
        ));
        let fill = space
            .memory
            .reserve(RegionId::Normal, va + 3 * PAGE_SIZE, 2 * PAGE_SIZE)
            .expect("reserve hole");
        let replacement = space
            .memory
            .map_lazy(fill, flags, flags)
            .expect("fill mapping hole");
        assert!(!Arc::ptr_eq(&mapping.map, &replacement.map));
        replacements.push(replacement);
        space.memory.backend().audit();
    }
    let failed = VirtAddr::wrap(BASE + 12 * PAGE_SIZE);
    let failed_reservation = space
        .memory
        .reserve(RegionId::Normal, failed, 2 * PAGE_SIZE)
        .expect("reserve failed map range");
    let one_frame = alloc::vec![Frame::allocate().expect("failure probe frame")];
    assert!(matches!(
        space.memory.map_frames(failed_reservation, one_frame, flags, flags),
        Err(crate::core::memory::manager::MapError::NotAligned)
    ));
    assert!(!space.memory.backend().overlaps(failed, 2 * PAGE_SIZE).unwrap());
    let reusable = space
        .memory
        .reserve(RegionId::Normal, failed, 2 * PAGE_SIZE)
        .expect("failed map unwinds reservation");
    space
        .memory
        .release_reservation(reusable)
        .expect("release failure probe");
    assert!(space.memory.backend().translate(failed).is_none());
    space.memory.backend().audit();
    for i in (0..COUNT).rev() {
        let map = &maps[COUNT - i - 1];
        space.memory.unmap(map).expect("remove split mapping");
        space.memory.backend().audit();
    }
    for mapping in &replacements {
        space.memory.unmap(mapping).expect("remove replacement mapping");
    }
    assert_eq!(space.memory.backend().table_count(), tables);
    let trampoline = space
        .memory
        .backend()
        .find(crate::machine::layout::TRAMPOLINE)
        .expect("seeded trampoline map");
    space
        .memory
        .backend()
        .protect(
            &trampoline,
            crate::machine::layout::TRAMPOLINE,
            PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::G,
        )
        .expect("top page protection");
    space
        .memory
        .backend()
        .unmap(&trampoline, crate::machine::layout::TRAMPOLINE, PAGE_SIZE)
        .expect("top page unmap");
    assert!(space.memory.backend().translate(crate::machine::layout::TRAMPOLINE).is_none());
    assert_eq!(space.memory.backend().table_count(), 1);
    space.memory.backend().audit();
}

pub fn large_pages() {
    use crate::core::memory::manager::BorrowedRegion;
    use crate::core::memory::manager::table;
    table::large_pages_accept();
    const MEGA: usize = 1 << 21;
    let va = VirtAddr::wrap(0x4000_0000);
    let ram = crate::machine::board::info().dram;
    let pa = PhysAddr::from_raw(ram.base.next_multiple_of(MEGA));
    assert!(pa.as_usize() + 2 * MEGA <= ram.base + ram.size);
    assert!(pa.as_usize().is_multiple_of(MEGA));
    let space = SpaceBuilder::user().build().expect("large page space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D | PteFlags::U;
    let before = space.memory.backend().table_count();
    let region = unsafe { BorrowedRegion::new(pa, 2 * MEGA) }.expect("reserved DRAM interval");
    let map = space
        .memory
        .backend()
        .map_borrowed(va, region, 0, 2 * MEGA, flags, flags)
        .expect("large borrowed maps");
    assert_eq!(
        space.memory.backend().table_count(),
        before + crate::core::memory::manager::mode::levels() - 2
    );
    for offset in [0, 13, PAGE_SIZE + 17, MEGA + 91, 2 * MEGA - 1] {
        assert_eq!(space.memory.backend().translate(va + offset).unwrap().0, pa + offset);
    }
    space
        .memory
        .backend()
        .protect(&map, va + PAGE_SIZE, PAGE_SIZE, flags - PteFlags::W)
        .expect("large space protect");
    assert!(
        !space
            .memory
            .backend()
            .translate(va + PAGE_SIZE)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(
        space
            .memory
            .backend()
            .translate(va + 2 * PAGE_SIZE)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    space
        .memory
        .backend()
        .unmap(&map, va + MEGA + PAGE_SIZE, PAGE_SIZE)
        .expect("large space hole");
    assert!(space.memory.backend().translate(va + MEGA + PAGE_SIZE).is_none());
    assert_eq!(
        space.memory.backend().translate(va + MEGA + 2 * PAGE_SIZE).unwrap().0,
        pa + MEGA + 2 * PAGE_SIZE
    );
    space.memory.backend().audit();
    for range in map.ranges().expect("large map ranges") {
        space
            .memory
            .backend()
            .unmap(&map, range.start, range.size.get())
            .expect("large space removal");
    }
    assert_eq!(space.memory.backend().table_count(), before);
    space.memory.backend().audit();
}
