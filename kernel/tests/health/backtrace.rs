#![cfg(debug_assertions)]

use alloc::sync::Arc;
use ::core::alloc::Layout;
use ::core::cell::Cell;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::{Access, SpaceKind, addr::VirtAddr};
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::entry::report::backtrace::{Backtrace, DEPTH, StackReader, WalkConfig};

fn word(space: &Space, address: usize, value: usize) {
    assert!(space.memory.copy_out(&value.to_le_bytes(), address));
}

pub fn walk() {
    let space = Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap();
    let mapping = space.memory.allocate(
        Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap(),
        Access::READ | Access::WRITE,
        None,
    ).unwrap();
    let base = mapping.address().as_usize();
    let fp = base + 64;
    for (frame, caller, pc) in [
        (fp, fp + 32, 0x1000),
        (fp + 32, fp + 64, 0x1000),
        (fp + 64, 0, 0x2000),
    ] {
        word(&space, frame - 16, caller);
        word(&space, frame - 8, pc);
    }
    let mut reader = StackReader::user(space.clone());
    let trace = Backtrace::walk(
        &mut reader, &WalkConfig::normal(SpaceKind::User, base + PAGE_SIZE),
        base, fp, None,
    );
    let frames = trace.frames();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].pc.as_usize(), 0x1000);
    assert_eq!(frames[0].sp.as_usize(), fp);
    assert_eq!(frames[0].fp.unwrap().as_usize(), fp + 32);
    assert_eq!(frames[1].pc.as_usize(), 0x2000);
    assert_eq!(frames[1].sp.as_usize(), fp + 64);
    assert!(frames[1].fp.is_none());

    let candidates = [0, 1, 0x1000, 0x1000, 0x1003, 0x2000, 0x1000];
    for (i, pc) in candidates.into_iter().enumerate() {
        word(&space, base + i * size_of::<usize>(), pc);
    }
    let cfg = WalkConfig::normal(SpaceKind::User, base + candidates.len() * size_of::<usize>());
    let trace = Backtrace::walk(&mut reader, &cfg, base, base - 8, Some(&|_| true));
    let pcs: alloc::vec::Vec<_> = trace.frames().iter().map(|frame| frame.pc.as_usize()).collect();
    assert_eq!(pcs, [0x1000, 0x2000, 0x1000]);

    for i in 0..DEPTH * 2 {
        word(&space, base + i * size_of::<usize>(), 0x1000 + i * 4);
    }
    let calls = Cell::new(0usize);
    let code = |_: usize| {
        calls.set(calls.get() + 1);
        true
    };
    let cfg = WalkConfig::normal(SpaceKind::User, base + DEPTH * 2 * size_of::<usize>());
    let trace = Backtrace::walk(&mut reader, &cfg, base, base - 8, Some(&code));
    assert_eq!(trace.frames().len(), DEPTH);
    assert_eq!(calls.get(), DEPTH);
    for (i, frame) in trace.frames().iter().enumerate() {
        assert_eq!(frame.pc.as_usize(), 0x1000 + i * 4);
        assert_eq!(frame.space, SpaceKind::User);
    }
    space.memory.unmap(&mapping).unwrap();

    let mapping = space.memory.allocate(
        Layout::from_size_align(3 * PAGE_SIZE, PAGE_SIZE).unwrap(),
        Access::READ | Access::WRITE,
        None,
    ).unwrap();
    let base = mapping.address().as_usize();
    let from = base + PAGE_SIZE - 8;
    word(&space, from, 0x1000);
    word(&space, base + 2 * PAGE_SIZE, 0x2000);
    space.memory.backend().unmap(
        &mapping.map, VirtAddr::wrap(base + PAGE_SIZE), PAGE_SIZE,
    ).unwrap();
    let mut reader = StackReader::user(space.clone());
    let mut cfg = WalkConfig {
        world: SpaceKind::User,
        gaps: false,
        ceiling: base + 2 * PAGE_SIZE + 8,
    };
    let trace = Backtrace::walk(&mut reader, &cfg, from, from - 8, Some(&|_| true));
    assert_eq!(trace.frames().len(), 1);
    assert_eq!(trace.frames()[0].pc.as_usize(), 0x1000);
    cfg.gaps = true;
    let trace = Backtrace::walk(&mut reader, &cfg, from, from - 8, Some(&|_| true));
    assert_eq!(trace.frames().len(), 2);
    assert_eq!(trace.frames()[1].pc.as_usize(), 0x2000);
    space.memory.unmap(&mapping).unwrap();
}
