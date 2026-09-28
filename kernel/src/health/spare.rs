#![cfg(debug_assertions)]

use core::alloc::{Allocator, Layout};
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::hart;
use crate::memory::allocator::spare;
use crate::memory::allocator::spare::DUMP_BUDGET;
use crate::memory::allocator::statistics;
use crate::runtime::diagnose::trace;

pub fn accept() {
    let h = hart::hart_count();
    let ring = trace::ring_bytes(h);

    crate::expect!(
        statistics::spare_occupied() >= ring,
        "spare: ring {ring} B not resident (occupied {})",
        statistics::spare_occupied()
    );
    crate::expect!(
        statistics::spare_available() >= DUMP_BUDGET,
        "spare: dump budget {DUMP_BUDGET} B not reserved (available {})",
        statistics::spare_available()
    );

    let step = Layout::from_size_align(1024, 16).unwrap();
    let before = (statistics::spare_occupied(), statistics::spare_available());
    let mut held: Vec<NonNull<[u8]>> = Vec::new();
    while let Ok(b) = spare::spare().allocate(step) {
        held.push(b)
    }
    crate::expect!(
        spare::spare().allocate(step).is_err(),
        "spare: drill did not reach exhaustion (available {})",
        statistics::spare_available()
    );
    for b in held.iter().rev() {
        unsafe { spare::spare().deallocate(b.cast(), step) };
    }
    let after = (statistics::spare_occupied(), statistics::spare_available());
    crate::expect!(
        after.1 == before.1,
        "spare: drill leaked budget (available {0} → {1})",
        before.1,
        after.1
    );
    crate::expect!(
        after.0 == before.0,
        "spare: drill left residue (occupied {0}), want {1}",
        after.0,
        before.0
    );
}