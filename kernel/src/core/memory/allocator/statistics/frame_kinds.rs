use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::core::memory::allocator::InitError;
use crate::machine::sync::OnceLock;

use super::kind::Kind;

struct FrameKinds {
    ptr: *mut Kind,
    len: usize,
}

unsafe impl Send for FrameKinds {}
unsafe impl Sync for FrameKinds {}

static FRAME_KINDS: OnceLock<FrameKinds> = OnceLock::new();

pub(crate) fn install_frame_kinds(frames: usize) -> Result<(), InitError> {
    let mut v: Vec<Kind> = Vec::new();
    v.try_reserve(frames)
        .map_err(|_| InitError::OutOfMemory)?;
    v.resize(frames, Kind::Plain);
    let slice: &'static mut [Kind] = Box::leak(v.into_boxed_slice());
    let h = FrameKinds {
        ptr: slice.as_mut_ptr(),
        len: slice.len(),
    };
    FRAME_KINDS
        .set(h)
        .map_err(|_| InitError::AlreadyInitialized)
}

fn frame_kind_table() -> &'static mut [Kind] {
    let h = FRAME_KINDS
        .get()
        .expect("frame kinds not installed (frame::init 应先调 install_frame_kinds)");
    unsafe { core::slice::from_raw_parts_mut(h.ptr, h.len) }
}

pub(super) fn note_frame_kind(index: usize, frames: usize, k: Kind) {
    frame_kind_table()[index..index + frames].fill(k);
}

pub(super) fn frame_kind_at(index: usize) -> Kind {
    frame_kind_table()[index]
}
