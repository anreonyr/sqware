use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState};
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
                unreachable!("Memory 五格不该见到这一枚 MapError")
            }
        }
    }
}

pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    match call {
        MemoryCall::Allocate { size } => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            match HeapWindow::allocate(&ident.team.space, size) {
                Ok(span) => frame.gpr.set_x(Gprs::A0, span.va.as_usize()),
                Err(e) => {
                    ret_err(frame, MemoryFail::from(e));
                }
            }
        }
        MemoryCall::Deallocate { addr, size } => {
            let addr = addr.get();
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let ok = HeapWindow::deallocate(&ident.team.space, KVirt::from_raw(addr), size);
            frame.gpr.set_x(
                Gprs::A0,
                if ok { 0 } else { MemoryFail::Denied.code() as usize },
            );
        }
        MemoryCall::Mmap { size, at } => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let fixed = at.get();
            let va = {
                let s = &ident.team.space;
                if fixed == 0 {
                    ShareWindow::mmap(s, size).map(|span| span.va)
                } else {
                    let flags = s.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
                    s.map(KVirt::from_raw(fixed), size, flags, Some(Pending::Lazy))
                        .map(|()| KVirt::from_raw(fixed))
                }
            };
            match va {
                Ok(va) => frame.gpr.set_x(Gprs::A0, va.as_usize()),
                Err(e) => {
                    ret_err(frame, MemoryFail::from(e));
                }
            }
        }
        MemoryCall::Munmap { addr, size } => {
            let addr = KVirt::from_raw(addr.get());
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                if ShareWindow::munmap(s, addr, size) {
                    true
                } else if s.pending_state(addr) != PendingState::Absent {
                    s.unmap(addr, size).is_ok()
                } else {
                    false
                }
            };
            frame.gpr.set_x(
                Gprs::A0,
                if ok { 0 } else { MemoryFail::Denied.code() as usize },
            );
        }
        MemoryCall::Mprotect { addr, size, flags } => {
            let addr = KVirt::from_raw(addr.get());
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let ok = match PteFlags::from_bits(flags) {
                Some(f) => ident.team.space.protect(addr, size, f).is_ok(),
                None => false,
            };
            frame.gpr.set_x(
                Gprs::A0,
                if ok { 0 } else { MemoryFail::Denied.code() as usize },
            );
        }
    }
}