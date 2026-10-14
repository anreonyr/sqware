use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::core::memory::allocator::InitError;
use crate::machine::sync::OnceLock;

use super::kind::Kind;

pub(super) const BLOCK_KIND_SHIFT: usize = 7;

struct BlockKinds {
    ptr: *mut Kind,
    len: usize,
    base: usize,
}

unsafe impl Send for BlockKinds {}
unsafe impl Sync for BlockKinds {}

static BLOCK_KINDS: OnceLock<BlockKinds> = OnceLock::new();

pub(crate) fn install_block_kinds(base: usize, len: usize) -> Result<(), InitError> {
    let slots = len >> BLOCK_KIND_SHIFT;
    let mut v: Vec<Kind> = Vec::new();
    v.try_reserve(slots)
        .map_err(|_| InitError::OutOfMemory)?;
    v.resize(slots, Kind::Plain);
    let slice: &'static mut [Kind] = Box::leak(v.into_boxed_slice());
    let h = BlockKinds {
        ptr: slice.as_mut_ptr(),
        len: slice.len(),
        base,
    };
    BLOCK_KINDS
        .set(h)
        .map_err(|_| InitError::AlreadyInitialized)
}

fn block_kind_table() -> (&'static mut [Kind], usize) {
    let h = BLOCK_KINDS
        .get()
        .expect("block kinds not installed (block::init 应先调 install_block_kinds)");
    let t = unsafe { core::slice::from_raw_parts_mut(h.ptr, h.len) };
    (t, h.base)
}

pub(super) fn note_block_kind(addr: usize, power: usize, k: Kind) {
    if power < BLOCK_KIND_SHIFT {
        return;
    }
    let (t, base) = block_kind_table();
    let first = (addr - base) >> BLOCK_KIND_SHIFT;
    t[first..first + (1 << (power - BLOCK_KIND_SHIFT))].fill(k);
}

pub(super) fn block_kind_at(addr: usize, power: usize) -> Kind {
    if power < BLOCK_KIND_SHIFT {
        return Kind::Plain;
    }
    let (t, base) = block_kind_table();
    t[(addr - base) >> BLOCK_KIND_SHIFT]
}
