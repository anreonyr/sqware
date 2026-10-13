#![cfg(debug_assertions)]

use ::core::alloc::Layout;

use env::MemoryFail;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::Access;
use crate::core::memory::manager::MapError;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::work::unit::space::SpaceBuilder;

fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).expect("manager_api: layout")
}

pub fn placement() {
    let space = SpaceBuilder::user()
        .build()
        .expect("manager_api: user space");
    for request in [
        layout(1, 1),
        layout(PAGE_SIZE - 1, 1),
        layout(PAGE_SIZE, 1),
        layout(PAGE_SIZE + 1, 1),
        layout(0, 1),
        layout(1, 2 * PAGE_SIZE),
    ] {
        let mapping = space
            .memory
            .allocate(request, Access::READ | Access::WRITE, None)
            .expect("manager_api: allocate");
        assert_eq!(mapping.address().as_usize() % request.align(), 0);
        assert!(mapping.size() >= request.size());
        assert_eq!(mapping.size() % PAGE_SIZE, 0);
        space.memory.unmap(&mapping).expect("manager_api: release");
    }

    let request = layout(1, 1);
    let first = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: first gap");
    let second = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: adjacent gap");
    assert_eq!(
        second.address().as_usize(),
        first.address().as_usize() + first.size()
    );
    let lowest = first.address();
    space.memory.unmap(&first).expect("manager_api: return first");
    let reused = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: lowest gap reuse");
    assert_eq!(reused.address(), lowest);
    space.memory.unmap(&second).expect("manager_api: return second");
    space.memory.unmap(&reused).expect("manager_api: return reused");

    let probe = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: probe");
    let base = probe.address().as_usize();
    space
        .memory
        .unmap(&probe)
        .expect("manager_api: release probe");

    assert!(matches!(
        space.memory.allocate(
            request,
            Access::READ | Access::WRITE,
            Some(VirtAddr::wrap(base + 8))
        ),
        Err(MemoryFail::NotAligned)
    ));
    assert!(matches!(
        space.memory.allocate(
            layout(1, 2 * PAGE_SIZE),
            Access::READ | Access::WRITE,
            Some(VirtAddr::wrap(base | PAGE_SIZE))
        ),
        Err(MemoryFail::NotAligned)
    ));

    let fixed = space
        .memory
        .allocate(
            request,
            Access::READ | Access::WRITE,
            Some(VirtAddr::wrap(base)),
        )
        .expect("manager_api: fixed placement");
    assert_eq!(fixed.address().as_usize(), base);
    assert!(matches!(
        space.memory.allocate(
            request,
            Access::READ | Access::WRITE,
            Some(VirtAddr::wrap(base))
        ),
        Err(MemoryFail::NoRegion)
    ));
    space
        .memory
        .unmap(&fixed)
        .expect("manager_api: release fixed");
}

pub fn receipt() {
    let space = SpaceBuilder::user()
        .build()
        .expect("manager_api: user space");
    let other = SpaceBuilder::supervisor()
        .build()
        .expect("manager_api: peer space");
    let request = layout(PAGE_SIZE, 1);
    let held = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: allocate");
    assert_eq!(held.size(), PAGE_SIZE);
    let stray = other
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: peer allocate");
    assert_eq!(space.memory.unmap(&stray), Err(MapError::NoRegion));
    assert_eq!(other.memory.unmap(&stray), Ok(()));

    let at = held.address();
    space.memory.dispose(held).expect("manager_api: discard");
    let again = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, Some(at))
        .expect("manager_api: reuse after discard");
    space.memory.unmap(&again).expect("manager_api: release");

    crate::core::memory::manager::memory::fail_cleanup_prepare();
    assert!(matches!(
        space
            .memory
            .allocate(request, Access::READ | Access::WRITE, Some(at)),
        Err(MemoryFail::OoM)
    ));
    let recovered = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, Some(at))
        .expect("manager_api: failed mapping left no address");
    space
        .memory
        .unmap(&recovered)
        .expect("manager_api: release recovered");
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
        let access = Access::from_bits(bits).expect("manager_api: logical bits");
        assert_eq!(Access::from_wire(bits << 1), Some(access));
        assert_eq!(access.bits(), bits);
    }
}

pub fn sharing() {
    let space = SpaceBuilder::user()
        .build()
        .expect("manager_api: user space");
    let request = layout(PAGE_SIZE, 1);
    let source = space
        .memory
        .allocate(request, Access::READ | Access::WRITE, None)
        .expect("manager_api: source");
    let at = source.address();
    assert!(space.memory.copy_out(b"origin", at.as_usize()));

    let aligned = layout(PAGE_SIZE, 2 * PAGE_SIZE);
    let misaligned = Some(VirtAddr::wrap(at.as_usize() | PAGE_SIZE));
    assert!(matches!(
        space.memory.allocate_lazy(
            aligned,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
            misaligned,
        ),
        Err(MemoryFail::NotAligned)
    ));
    let lazy_aligned = space
        .memory
        .allocate_lazy(
            aligned,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
            None,
        )
        .expect("manager_api: aligned lazy placement");
    assert_eq!(lazy_aligned.address().as_usize() % aligned.align(), 0);
    space
        .memory
        .unmap(&lazy_aligned)
        .expect("manager_api: return aligned lazy placement");
    assert!(matches!(
        space.memory.view(
            &source,
            0,
            aligned,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
            misaligned,
        ),
        Err(MemoryFail::NotAligned)
    ));
    let view_aligned = space
        .memory
        .view(
            &source,
            0,
            aligned,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
            None,
        )
        .expect("manager_api: aligned view placement");
    assert_eq!(view_aligned.address().as_usize() % aligned.align(), 0);
    space
        .memory
        .unmap(&view_aligned)
        .expect("manager_api: return aligned view placement");

    let view = space
        .memory
        .view(
            &source,
            0,
            request,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
            None,
        )
        .expect("manager_api: view");
    let shared = view.address();
    let mut bytes = [0u8; 6];
    assert!(space.memory.copy_in(&mut bytes, shared.as_usize()));
    assert_eq!(&bytes, b"origin");
    assert!(space.memory.copy_out(b"shared", at.as_usize()));
    assert!(space.memory.copy_in(&mut bytes, shared.as_usize()));
    assert_eq!(&bytes, b"shared");

    assert!(matches!(
        space
            .memory
            .protect_range(shared, PAGE_SIZE, Access::READ | Access::EXECUTE),
        Err(MemoryFail::WidenDenied)
    ));
    space
        .memory
        .protect_range(shared, PAGE_SIZE, Access::READ)
        .expect("manager_api: narrow view");
    assert!(!space.memory.copy_out(b"x", shared.as_usize()));
    space
        .memory
        .protect_range(at, PAGE_SIZE, Access::READ | Access::EXECUTE)
        .expect("manager_api: source ceiling holds execute");

    space
        .memory
        .unmap(&view)
        .expect("manager_api: deallocate view");
    space
        .memory
        .dispose(source)
        .expect("manager_api: discard source");

    let lazy = space
        .memory
        .allocate_lazy(
            request,
            Access::READ | Access::WRITE,
            Access::READ | Access::WRITE,
            None,
        )
        .expect("manager_api: lazy");
    let derived = space
        .memory
        .view(
            &lazy,
            0,
            request,
            Access::READ | Access::WRITE,
            Access::READ,
            None,
        )
        .expect("manager_api: derive lazy source");
    assert!(space.memory.copy_out(b"lazy..", lazy.address().as_usize()));
    let mut lazy_bytes = [0u8; 6];
    assert!(
        space
            .memory
            .copy_in(&mut lazy_bytes, derived.address().as_usize())
    );
    assert_eq!(&lazy_bytes, b"lazy..");
    space
        .memory
        .unmap(&derived)
        .expect("manager_api: release derived");
    space
        .memory
        .unmap(&lazy)
        .expect("manager_api: release lazy");
}
