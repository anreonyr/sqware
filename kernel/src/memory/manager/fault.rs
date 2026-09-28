use fack::prelude::Error;
use log::{error, info};
use riscv::register::{scause, sepc, stval};

use crate::memory::PAGE_SIZE;

use super::{addr::VirtAddr, entry::PteFlags};
use crate::work::unit::space::{PendingState, Space};

#[derive(Debug)]
pub struct PageFault {
    pub addr: VirtAddr,
    pub pc: usize,
    pub kind: FaultKind,
}

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FaultKind {
    #[error("Execute OOM Instruction")]
    Instruction,
    #[error("Load OOM Data")]
    Load,
    #[error("Store OOM Data")]
    Store,
}

impl PageFault {
    pub unsafe fn capture() -> Self {
        let code = scause::read().code();
        let kind = match code {
            12 => FaultKind::Instruction,
            13 => FaultKind::Load,
            15 => FaultKind::Store,
            _ => panic!("capture() called on non-page-fault scause={}", code),
        };

        Self {
            addr: VirtAddr::from_raw(stval::read()),
            pc: sepc::read(),
            kind,
        }
    }
}

fn resolve_anonymous(fault: &PageFault, space: &Space) -> bool {
    let vaddr = fault.addr.page_align();

    match space.materialize(vaddr, PAGE_SIZE) {
        Ok(()) => true,
        Err(e) => {
            error!("failed to resolve page fault: {:?}", e);
            false
        }
    }
}

fn satisfies(flags: PteFlags, kind: FaultKind) -> bool {
    let need = match kind {
        FaultKind::Instruction => PteFlags::X,
        FaultKind::Load => PteFlags::R,
        FaultKind::Store => PteFlags::W,
    };
    flags.contains(PteFlags::V | need)
}

pub fn handle_page_fault(fault: &PageFault, space: &Space) -> bool {
    if let Some((_paddr, flags)) = space.translate(fault.addr)
        && satisfies(flags, fault.kind)
    {
        info!(
            "page fault resolved by re-walk: {:?} at {:?}",
            fault.kind, fault.addr
        );
        return true;
    }

    if fault.addr.is_user() {
        match space.pending_state(fault.addr) {
            PendingState::Lazy => {
                return resolve_anonymous(fault, space);
            }
            PendingState::Guard => {
                error!(
                    "reserved region access: {:?} at {:?}, pc={:#x}",
                    fault.kind, fault.addr, fault.pc
                );
                return false;
            }
            PendingState::Materialized => {
                match space.translate(fault.addr) {
                    Some((_paddr, flags)) if !satisfies(flags, fault.kind) => {
                        error!(
                            "permission violation on materialized map: {:?} at {:?}, pc={:#x}",
                            fault.kind, fault.addr, fault.pc
                        );
                    }
                    Some((_paddr, _flags)) => {
                        error!(
                            "materialized map re-walk inconsistency: {:?} at {:?}, pc={:#x}",
                            fault.kind, fault.addr, fault.pc
                        );
                    }
                    None => {
                        error!(
                            "materialized map missing PTE (kernel bug): {:?} at {:?}, pc={:#x}",
                            fault.kind, fault.addr, fault.pc
                        );
                    }
                }
                return false;
            }
            PendingState::Absent => {
                error!(
                    "no map for user page fault: {:?} at {:?}, pc={:#x}",
                    fault.kind, fault.addr, fault.pc
                );
                return false;
            }
        }
    }

    error!(
        "unhandled kernel page fault: {:?} at {:?}, pc={:#x}",
        fault.kind, fault.addr, fault.pc
    );
    error!("kernel page fault — this is a bug (kernel pages must be pre-mapped)");
    false
}