#![cfg(debug_assertions)]

use ::core::alloc::Layout;

use env::MemoryFail;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::Access;
use crate::core::memory::nest::NestError;
use crate::core::work::unit::space::SpaceBuilder;

use super::support;

fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).expect("memory: layout")
}

fn rw() -> Access {
    Access::READ | Access::WRITE
}

pub fn placement() {
    let space = SpaceBuilder::user().build().expect("memory: user space");
    for request in [
        layout(1, 1),
        layout(PAGE_SIZE - 1, 1),
        layout(PAGE_SIZE, 1),
        layout(PAGE_SIZE + 1, 1),
        layout(0, 1),
        layout(1, 2 * PAGE_SIZE),
    ] {
        let view = support::allocate(&space, request, rw(), None).expect("memory: allocate");
        assert_eq!(view.initial_start().as_usize() % request.align(), 0);
        assert!(view.initial_size() >= request.size());
        assert_eq!(view.initial_size() % PAGE_SIZE, 0);
        space.nest.detach(view.whole()).expect("memory: release");
    }

    let request = layout(1, 1);
    let first = support::allocate(&space, request, rw(), None).expect("memory: first gap");
    let second = support::allocate(&space, request, rw(), None).expect("memory: adjacent gap");
    assert_eq!(
        second.initial_start().as_usize(),
        first.initial_start().as_usize() + first.initial_size()
    );
    let lowest = first.initial_start();
    space
        .nest
        .detach(first.whole())
        .expect("memory: return first");
    let reused = support::allocate(&space, request, rw(), None).expect("memory: lowest gap reuse");
    assert_eq!(reused.initial_start(), lowest);
    space
        .nest
        .detach(second.whole())
        .expect("memory: return second");
    space
        .nest
        .detach(reused.whole())
        .expect("memory: return reused");

    let probe = support::allocate(&space, request, rw(), None).expect("memory: probe");
    let base = probe.initial_start().as_usize();
    space
        .nest
        .detach(probe.whole())
        .expect("memory: release probe");

    assert!(matches!(
        support::allocate(&space, request, rw(), Some(VirtAddr::wrap(base + 8))),
        Err(MemoryFail::NotAligned)
    ));
    assert!(matches!(
        support::allocate(
            &space,
            layout(1, 2 * PAGE_SIZE),
            rw(),
            Some(VirtAddr::wrap(base | PAGE_SIZE))
        ),
        Err(MemoryFail::NotAligned)
    ));

    let fixed = support::allocate(&space, request, rw(), Some(VirtAddr::wrap(base)))
        .expect("memory: fixed placement");
    assert_eq!(fixed.initial_start().as_usize(), base);
    assert!(matches!(
        support::allocate(&space, request, rw(), Some(VirtAddr::wrap(base))),
        Err(MemoryFail::NoRegion)
    ));
    space
        .nest
        .detach(fixed.whole())
        .expect("memory: release fixed");
}

pub fn view_identity() {
    let space = SpaceBuilder::user().build().expect("memory: user space");
    let other = SpaceBuilder::supervisor()
        .build()
        .expect("memory: peer space");
    let request = layout(PAGE_SIZE, 1);
    let held = support::allocate(&space, request, rw(), None).expect("memory: allocate");
    assert_eq!(held.initial_size(), PAGE_SIZE);
    let stray = support::allocate(&other, request, rw(), None).expect("memory: peer allocate");
    assert_eq!(space.nest.detach(stray.whole()), Err(NestError::NoRegion));
    assert_eq!(other.nest.detach(stray.whole()), Ok(()));

    let at = held.initial_start();
    space.nest.dispose(held).expect("memory: discard");
    let again =
        support::allocate(&space, request, rw(), Some(at)).expect("memory: reuse after discard");
    space.nest.detach(again.whole()).expect("memory: release");

    crate::core::memory::nest::fail_cleanup_prepare();
    assert!(matches!(
        support::allocate(&space, request, rw(), Some(at)),
        Err(MemoryFail::OoM)
    ));
    let recovered = support::allocate(&space, request, rw(), Some(at))
        .expect("memory: failed mapping left no address");
    space
        .nest
        .detach(recovered.whole())
        .expect("memory: release recovered");
}

pub fn access_wire() {
    assert_eq!(Access::from_wire(0), Some(Access::empty()));
    assert_eq!(Access::from_wire(2), Some(Access::READ));
    assert_eq!(Access::from_wire(4), Some(Access::WRITE));
    assert_eq!(Access::from_wire(8), Some(Access::EXECUTE));
    assert_eq!(
        Access::from_wire(14),
        Some(Access::READ | Access::WRITE | Access::EXECUTE)
    );
    for illegal in [1u64, 15, 16, 32, 64, 128, 256, 1 << 63] {
        assert_eq!(Access::from_wire(illegal), None);
    }
    for bits in 0..8u64 {
        let access = Access::from_bits(bits).expect("memory: logical bits");
        assert_eq!(Access::from_wire(bits << 1), Some(access));
        assert_eq!(access.bits(), bits);
    }
}

pub fn source_sharing() {
    let space = SpaceBuilder::user().build().expect("memory: user space");
    let request = layout(PAGE_SIZE, 1);
    let source = support::allocate(&space, request, rw(), None).expect("memory: source");
    let at = source.initial_start();
    assert!(space.copy_out(b"origin", at));

    let aligned = layout(PAGE_SIZE, 2 * PAGE_SIZE);
    let misaligned = Some(VirtAddr::wrap(at.as_usize() | PAGE_SIZE));
    assert!(matches!(
        support::allocate_lazy(&space, aligned, rw(), rw(), misaligned),
        Err(MemoryFail::NotAligned)
    ));
    let lazy_aligned = support::allocate_lazy(&space, aligned, rw(), rw(), None)
        .expect("memory: aligned lazy placement");
    assert_eq!(lazy_aligned.initial_start().as_usize() % aligned.align(), 0);
    space
        .nest
        .detach(lazy_aligned.whole())
        .expect("memory: return aligned lazy placement");
    assert!(matches!(
        support::derive(
            &space,
            &source,
            0,
            basic(aligned),
            aligned.align(),
            rw(),
            rw(),
            misaligned,
        ),
        Err(MemoryFail::NotAligned)
    ));
    let view_aligned = support::derive(
        &space,
        &source,
        0,
        basic(aligned),
        aligned.align(),
        rw(),
        rw(),
        None,
    )
    .expect("memory: aligned view placement");
    assert_eq!(view_aligned.initial_start().as_usize() % aligned.align(), 0);
    space
        .nest
        .detach(view_aligned.whole())
        .expect("memory: return aligned view placement");

    let view = support::derive(
        &space,
        &source,
        0,
        basic(request),
        request.align(),
        rw(),
        rw(),
        None,
    )
    .expect("memory: view");
    let shared = view.initial_start();
    let mut bytes = [0u8; 6];
    assert!(space.copy_in(&mut bytes, shared));
    assert_eq!(&bytes, b"origin");
    assert!(space.copy_out(b"shared", at));
    assert!(space.copy_in(&mut bytes, shared));
    assert_eq!(&bytes, b"shared");

    assert!(matches!(
        support::protect(&space, shared, PAGE_SIZE, Access::READ | Access::EXECUTE),
        Err(MemoryFail::WidenDenied)
    ));
    support::protect(&space, shared, PAGE_SIZE, Access::READ).expect("memory: narrow view");
    assert!(!space.copy_out(b"x", shared));
    support::protect(&space, at, PAGE_SIZE, Access::READ | Access::EXECUTE)
        .expect("memory: source ceiling holds execute");

    space
        .nest
        .detach(view.whole())
        .expect("memory: deallocate view");
    space.nest.dispose(source).expect("memory: discard source");

    let lazy = support::allocate_lazy(&space, request, rw(), rw(), None).expect("memory: lazy");
    let derived = support::derive(
        &space,
        &lazy,
        0,
        basic(request),
        request.align(),
        rw(),
        Access::READ,
        None,
    )
    .expect("memory: derive lazy source");
    assert!(space.copy_out(b"lazy..", lazy.initial_start()));
    let mut lazy_bytes = [0u8; 6];
    assert!(space.copy_in(&mut lazy_bytes, derived.initial_start()));
    assert_eq!(&lazy_bytes, b"lazy..");
    space
        .nest
        .detach(derived.whole())
        .expect("memory: release derived");
    space
        .nest
        .detach(lazy.whole())
        .expect("memory: release lazy");
}

/// 旧 `view` 入口的长度口径：最少 1 字节，向上取整到页。
fn basic(layout: Layout) -> usize {
    layout.size().max(1).next_multiple_of(PAGE_SIZE)
}
