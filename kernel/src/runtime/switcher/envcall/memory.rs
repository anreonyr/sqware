use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::Space;
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::task::TaskIdent;

use super::ret_err;

impl From<MapError> for MemoryFail {
    fn from(e: MapError) -> Self {
        match e {
            MapError::OutOfMemory => MemoryFail::OoM,
            MapError::NotAligned => MemoryFail::NotAligned,
            MapError::NoRegion => MemoryFail::NoRegion,
            MapError::AlreadyMapped => MemoryFail::AlreadyMapped,
            MapError::WidenDenied => MemoryFail::WidenDenied,
            MapError::NotMapped | MapError::SegmentMismatch | MapError::DramOverlap => {
                MemoryFail::Denied
            }
        }
    }
}

pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    let size = match call {
        MemoryCall::Allocate { size }
        | MemoryCall::Deallocate { size, .. }
        | MemoryCall::Mmap { size, .. }
        | MemoryCall::Munmap { size, .. }
        | MemoryCall::Mprotect { size, .. } => size,
    };
    let Some(size) = size.max(1).checked_next_multiple_of(PAGE_SIZE) else {
        ret_err(frame, MemoryFail::Denied);
        return;
    };
    let space = &ident.team.space;
    let result = match call {
        MemoryCall::Allocate { .. } => {
            HeapWindow::allocate(space, size).map(|span| span.va.as_usize())
        }
        MemoryCall::Mmap { at, .. } => {
            if at.get() == 0 {
                ShareWindow::mmap(space, size).map(|span| span.va.as_usize())
            } else if !Space::user_range(at.get(), size) {
                Err(MapError::NoRegion)
            } else {
                ShareWindow::mmap_at(space, KVirt::wrap(at.get()), size).map(|()| at.get())
            }
        }
        MemoryCall::Deallocate { addr, .. } => {
            if Space::user_range(addr.get(), size)
                && HeapWindow::deallocate(space, KVirt::wrap(addr.get()), size)
            {
                Ok(0)
            } else {
                Err(MapError::SegmentMismatch)
            }
        }
        MemoryCall::Munmap { addr, .. } => {
            if Space::user_range(addr.get(), size)
                && (ShareWindow::munmap(space, KVirt::wrap(addr.get()), size)
                    || space.unmap_user(addr.get(), size).is_ok())
            {
                Ok(0)
            } else {
                Err(MapError::SegmentMismatch)
            }
        }
        MemoryCall::Mprotect { addr, flags, .. } => {
            space.protect_user(addr.get(), size, flags).map(|()| 0)
        }
    };
    match result {
        Ok(value) => frame.gpr.set_x(Gprs::A0, value),
        Err(error) => {
            ret_err(frame, MemoryFail::from(error));
        }
    }
}
