#![cfg(debug_assertions)]

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::space::window::HeapWindow;

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

pub fn kernel_layout() {
    use crate::layout::{IMAGE_BASE, TRAMPOLINE};
    use crate::memory::manager::mode;
    use crate::work::unit::space::SegmentKind;

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
    let ram = crate::platform::machine::info().dram;
    let space = &crate::work::unit::team::kernel()
        .expect("kernel team")
        .space;
    let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
    space.with(|inner| {
        assert!(inner.holds(SegmentKind::Normal, ram.base, ram.size));
        for pa in (ram.base..ram.base + ram.size).step_by(PAGE_SIZE) {
            let expected = if (text_start..text_end).contains(&pa) {
                PteFlags::R | PteFlags::X
            } else if (rodata_start..image_end).contains(&pa) {
                PteFlags::R
            } else {
                PteFlags::R | PteFlags::W
            };
            for offset in [0, mode::lower().as_usize()] {
                let va = VirtAddr::wrap(offset + pa);
                let trap_base = crate::runtime::switcher::trap::trap_stack();
                let guard = offset == 0
                    && (0..crate::hart::hart_count())
                        .any(|hart| pa == trap_base + hart * crate::layout::TRAP_STACK_SLOT_SIZE);
                if guard {
                    assert!(
                        inner.root.walk_ref(va).is_err(),
                        "trap stack physical guard"
                    );
                    continue;
                }
                let (physical, flags) = inner.root.walk_ref(va).expect("kernel RAM mapping");
                assert_eq!(physical, PhysAddr::from_raw(pa));
                assert_eq!(
                    (flags & rwx).bits(),
                    expected.bits(),
                    "kernel mapping at {va:?}"
                );
                assert!(flags.contains(PteFlags::G) && !flags.contains(PteFlags::U));
            }
        }
        let (pa, flags) = inner.root.walk_ref(TRAMPOLINE).expect("trampoline mapping");
        assert_eq!(pa, crate::layout::trampoline_pa());
        assert_eq!((flags & rwx).bits(), (PteFlags::R | PteFlags::X).bits());

        // A request larger than the lower gap must skip the reserved RAM range.
        let size = ram.base - IMAGE_BASE.as_usize() + PAGE_SIZE;
        let va = HeapWindow::locate(inner, size).expect("kernel virtual allocation");
        inner
            .allocate(SegmentKind::Normal, va.as_usize(), size)
            .expect("kernel virtual allocation");
        assert_eq!(va.as_usize(), ram.base + ram.size);
        assert!(inner.deallocate(SegmentKind::Normal, va.as_usize(), size));
        let va = HeapWindow::locate(inner, PAGE_SIZE).expect("kernel lower gap");
        inner
            .allocate(SegmentKind::Normal, va.as_usize(), PAGE_SIZE)
            .expect("kernel lower gap");
        assert_eq!(va, IMAGE_BASE);
        assert!(inner.deallocate(SegmentKind::Normal, va.as_usize(), PAGE_SIZE));
    });
}

pub fn window_layout() {
    use crate::layout::TASK_STACK_GUARD;
    use crate::memory::manager::mode;
    use crate::work::unit::space::window::{HeapWindow, StackWindow};
    use crate::work::unit::space::{PendingState, SegmentKind};

    const BASE: usize = 0x4000_0000;
    let edge = mode::upper().as_usize();
    let floor = BASE + 4 * PAGE_SIZE;
    let space = SpaceBuilder::user().build().expect("window space");
    space.with(|inner| {
        inner.dynamic(floor);
        assert!(
            inner
                .allocate(SegmentKind::Normal, BASE, PAGE_SIZE)
                .is_err()
        );
    });
    let tables = space.table_count();
    let heap = HeapWindow::allocate(&space, 2 * PAGE_SIZE).expect("heap above image");
    assert_eq!(heap.va.as_usize(), floor);
    let first = StackWindow::claim(&space, 2 * PAGE_SIZE).expect("first stack");
    let second = StackWindow::claim(&space, 2 * PAGE_SIZE).expect("second stack");
    assert_eq!(first.va.as_usize() + first.size.get(), edge);
    assert_eq!(
        second.va.as_usize() + second.size.get(),
        first.va.as_usize()
    );
    for stack in [first, second] {
        assert!(matches!(space.pending_state(stack.va), PendingState::Guard));
        assert!(space.translate(stack.va).is_none());
        for offset in (TASK_STACK_GUARD..stack.size.get()).step_by(PAGE_SIZE) {
            let (_, flags) = space.translate(stack.va + offset).expect("stack body");
            assert!(flags.contains(PteFlags::U | PteFlags::R | PteFlags::W));
            assert!(!flags.contains(PteFlags::X));
        }
    }
    space.release(first).expect("release top stack");
    let reused = StackWindow::claim(&space, 2 * PAGE_SIZE).expect("reuse top stack");
    assert_eq!(reused.va, first.va);
    space.release(reused).expect("release reused stack");
    space.release(second).expect("release second stack");
    space.release(heap).expect("release heap");
    assert_eq!(space.table_count(), tables);
    space.with(|inner| {
        let fixed = edge - 3 * PAGE_SIZE;
        inner
            .allocate(SegmentKind::Normal, fixed, 2 * PAGE_SIZE)
            .expect("fixed allocation");
        assert!(
            inner
                .allocate(SegmentKind::Normal, fixed + PAGE_SIZE, PAGE_SIZE)
                .is_err()
        );
        let top = StackWindow::locate(inner, PAGE_SIZE).expect("top gap");
        inner
            .allocate(SegmentKind::Normal, top.as_usize(), PAGE_SIZE)
            .expect("address allocation");
        assert_eq!(top.as_usize(), edge - PAGE_SIZE);
        let below = StackWindow::locate(inner, 2 * PAGE_SIZE).expect("skip fixed mapping");
        inner
            .allocate(SegmentKind::Normal, below.as_usize(), 2 * PAGE_SIZE)
            .expect("address allocation");
        assert_eq!(below.as_usize(), fixed - 2 * PAGE_SIZE);
        for (base, size) in [
            (fixed, 2 * PAGE_SIZE),
            (top.as_usize(), PAGE_SIZE),
            (below.as_usize(), 2 * PAGE_SIZE),
        ] {
            assert!(inner.deallocate(SegmentKind::Normal, base, size));
        }
        let size = edge - floor - PAGE_SIZE;
        let heap = HeapWindow::locate(inner, size).expect("fill lower gap");
        inner
            .allocate(SegmentKind::Normal, heap.as_usize(), size)
            .expect("address allocation");
        assert_eq!(heap.as_usize(), floor);
        assert!(StackWindow::locate(inner, 2 * PAGE_SIZE).is_err());
        let stack = StackWindow::locate(inner, PAGE_SIZE).expect("last page");
        inner
            .allocate(SegmentKind::Normal, stack.as_usize(), PAGE_SIZE)
            .expect("address allocation");
        assert_eq!(stack.as_usize(), edge - PAGE_SIZE);
        assert!(HeapWindow::locate(inner, PAGE_SIZE).is_err());
        let segment = inner.user.as_mut().expect("segment");
        segment.prepare_cut().expect("retirement capacity");
        let ticket = segment.retire(stack.as_usize(), PAGE_SIZE);
        assert!(
            inner
                .allocate(SegmentKind::Normal, stack.as_usize(), PAGE_SIZE)
                .is_err()
        );
        assert!(HeapWindow::locate(inner, PAGE_SIZE).is_err());
        assert!(StackWindow::locate(inner, PAGE_SIZE).is_err());
        inner.user.as_mut().expect("segment").reclaim(ticket);
        let reclaimed = HeapWindow::locate(inner, PAGE_SIZE).expect("reclaimed address");
        inner
            .allocate(SegmentKind::Normal, reclaimed.as_usize(), PAGE_SIZE)
            .expect("address allocation");
        assert_eq!(reclaimed, stack);
        assert!(inner.deallocate(SegmentKind::Normal, reclaimed.as_usize(), PAGE_SIZE));
        assert!(inner.deallocate(SegmentKind::Normal, heap.as_usize(), size));
    });
    space.audit();
}

pub fn image_dynamic_region() {
    use crate::work::unit::space::window::HeapWindow;
    use crate::work::unit::space::{PendingState, SegmentKind};

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
        // A sparse image ending in four lazy BSS pages.
        put(&mut bytes, 72, 0x14000, 8);
        put(&mut bytes, 80, 4, 8);
        put(&mut bytes, 104, 3, 4);
        bytes
    }
    static IMAGE: Image = Image(fixture());
    let team = crate::work::unit::capsule::assemble(&IMAGE.0).expect("sparse capsule");
    assert!(matches!(
        team.space.pending_state(VirtAddr::wrap(0x17000)),
        PendingState::Lazy
    ));
    let heap = HeapWindow::allocate(&team.space, PAGE_SIZE).expect("heap after BSS");
    assert_eq!(heap.va.as_usize(), 0x18000);
    team.space.release(heap).expect("release image heap");
    // Removing image pages must not move the dynamic region's lower boundary.
    team.space
        .unmap_user(0x17000, PAGE_SIZE)
        .expect("unmap lazy image page");
    team.space
        .unmap_user(0x10000, PAGE_SIZE)
        .expect("unmap borrowed image page");
    team.space.with(|inner| {
        assert!(
            inner
                .allocate(SegmentKind::Normal, 0x12000, PAGE_SIZE)
                .is_err()
        );
        let va = HeapWindow::locate(inner, PAGE_SIZE).expect("dynamic mapping after BSS");
        inner
            .allocate(SegmentKind::Normal, va.as_usize(), PAGE_SIZE)
            .expect("address allocation");
        assert_eq!(va.as_usize(), 0x18000);
        assert!(inner.deallocate(SegmentKind::Normal, va.as_usize(), PAGE_SIZE));
    });
    team.space.audit();
}

pub fn interval_index() {
    use crate::memory::manager::MapError;
    use crate::work::unit::space::Pending;

    crate::work::unit::space::index_accept();
    const BASE: usize = 0x4000_0000;
    const COUNT: usize = 64;
    const STRIDE: usize = 16 * PAGE_SIZE;
    let space = SpaceBuilder::user().build().expect("interval space");
    let flags =
        space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
    let tables = space.table_count();
    for i in (0..COUNT).rev() {
        let va = VirtAddr::wrap(BASE + i * STRIDE);
        space
            .with(|inner| inner.map(va, 8 * PAGE_SIZE, flags, Some(Pending::Lazy)))
            .expect("indexed lazy region");
    }
    for i in 0..COUNT {
        let va = VirtAddr::wrap(BASE + i * STRIDE);
        space
            .protect_user(va.as_usize() + 2 * PAGE_SIZE, 2 * PAGE_SIZE, 2)
            .expect("split permissions");
        space
            .materialize(va + 2 * PAGE_SIZE, PAGE_SIZE)
            .expect("protected fault");
        let (_, pte) = space.translate(va + 2 * PAGE_SIZE).expect("protected page");
        assert!(pte.contains(PteFlags::R | PteFlags::U) && !pte.contains(PteFlags::W));
        space
            .materialize(va + 4 * PAGE_SIZE, PAGE_SIZE)
            .expect("writable fault");
        assert!(
            space
                .translate(va + 4 * PAGE_SIZE)
                .expect("writable page")
                .1
                .contains(PteFlags::W)
        );
        space.unmap(va, PAGE_SIZE).expect("move mapping start");
        space
            .unmap(va + 3 * PAGE_SIZE, 2 * PAGE_SIZE)
            .expect("cut across mappings");
        space
            .with(|inner| {
                assert!(!inner.overlaps(va, PAGE_SIZE));
                assert!(inner.overlaps(va + PAGE_SIZE, PAGE_SIZE));
                assert!(!inner.overlaps(va + 3 * PAGE_SIZE, 2 * PAGE_SIZE));
                inner.map(
                    va + 3 * PAGE_SIZE,
                    2 * PAGE_SIZE,
                    flags,
                    Some(Pending::Lazy),
                )
            })
            .expect("fill mapping hole");
        space.audit();
    }
    let failed = VirtAddr::wrap(BASE + 12 * PAGE_SIZE);
    space.with(|inner| {
        let mut pages = 0;
        let result = inner.claim(failed, 2 * PAGE_SIZE, flags, || {
            pages += 1;
            if pages == 2 {
                Err(MapError::OutOfMemory)
            } else {
                crate::work::unit::space::inner_frame()
            }
        });
        assert!(matches!(result, Err(MapError::OutOfMemory)));
        assert!(!inner.overlaps(failed, 2 * PAGE_SIZE));
    });
    assert!(space.translate(failed).is_none());
    space.audit();
    for i in (0..COUNT).rev() {
        space
            .unmap(VirtAddr::wrap(BASE + i * STRIDE), STRIDE)
            .expect("remove split region");
        space.audit();
    }
    assert_eq!(space.table_count(), tables);
    // The last virtual page must also support protection and full removal.
    space
        .with_shootdown(|inner| {
            inner.protect(
                crate::layout::TRAMPOLINE,
                PAGE_SIZE,
                PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::G,
                false,
            )
        })
        .expect("top page shootdown")
        .expect("top page protection");
    space
        .unmap(crate::layout::TRAMPOLINE, PAGE_SIZE)
        .expect("top page unmap");
    assert!(space.translate(crate::layout::TRAMPOLINE).is_none());
    assert_eq!(space.table_count(), 1);
    space.audit();
}

pub fn large_pages() {
    use crate::memory::manager::table;
    table::large_pages_accept();
    const MEGA: usize = 1 << 21;
    let va = VirtAddr::wrap(0x4000_0000);
    let pa = PhysAddr::from_raw(0x8000_0000);
    let space = SpaceBuilder::user().build().expect("large page space");
    let flags =
        space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
    let before = space.table_count();
    space
        .borrow(va, pa, 2 * MEGA, flags)
        .expect("large borrowed maps");
    assert_eq!(
        space.table_count(),
        before + crate::memory::manager::mode::levels() - 2
    );
    for offset in [0, 13, PAGE_SIZE + 17, MEGA + 91, 2 * MEGA - 1] {
        assert_eq!(space.translate(va + offset).unwrap().0, pa + offset);
    }
    space
        .protect(va + PAGE_SIZE, PAGE_SIZE, flags - PteFlags::W)
        .expect("large space protect");
    assert!(
        !space
            .translate(va + PAGE_SIZE)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(
        space
            .translate(va + 2 * PAGE_SIZE)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    space
        .unmap(va + MEGA + PAGE_SIZE, PAGE_SIZE)
        .expect("large space hole");
    assert!(space.translate(va + MEGA + PAGE_SIZE).is_none());
    assert_eq!(
        space.translate(va + MEGA + 2 * PAGE_SIZE).unwrap().0,
        pa + MEGA + 2 * PAGE_SIZE
    );
    space.audit();
    space.unmap(va, 2 * MEGA).expect("large space removal");
    assert_eq!(space.table_count(), before);
    space.audit();
}
