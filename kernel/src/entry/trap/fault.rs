use log::{error, info};
use riscv::register::{scause, sepc, stval};

use crate::core::memory::PAGE_SIZE;

use crate::core::memory::manager::{FaultKind, addr::VirtAddr, entry::PteFlags};
use crate::core::work::unit::space::Space;

#[derive(Debug)]
pub struct PageFault {
    pub addr: VirtAddr,
    pub pc: usize,
    pub kind: FaultKind,
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

fn satisfies(flags: PteFlags, kind: FaultKind) -> bool {
    let need = match kind {
        FaultKind::Instruction => PteFlags::X,
        FaultKind::Load => PteFlags::R,
        FaultKind::Store => PteFlags::W,
    };
    flags.contains(PteFlags::V | need)
}

pub fn handle_page_fault(fault: &PageFault, space: &Space) -> bool {
    if space.tree.has_pending_invalidation() {
        if let Err(error) = space.tree.retry_invalidation() {
            error!("page fault waits for invalidation: {:?}", error);
            return false;
        }
    }
    if let Some((_physical, flags)) = space.tree.translate(fault.addr) {
        if satisfies(flags, fault.kind) {
            info!(
                "page fault resolved by re-walk: {:?} at {:?}",
                fault.kind, fault.addr
            );
            return true;
        }
        error!(
            "page fault permission violation: {:?} at {:?}",
            fault.kind, fault.addr
        );
        return false;
    }
    if fault.addr.is_user() {
        let Some(map) = space.tree.find(fault.addr) else {
            error!(
                "unmapped or guard access: {:?} at {:?}, pc={:#x}",
                fault.kind, fault.addr, fault.pc
            );
            return false;
        };
        let permitted = map.ranges().is_ok_and(|ranges| {
            ranges.iter().any(|range| {
                range.start <= fault.addr
                    && range.end() >= fault.addr.as_usize()
                    && satisfies(range.current, fault.kind)
            })
        });
        if !permitted {
            error!(
                "lazy page permission violation: {:?} at {:?}",
                fault.kind, fault.addr
            );
            return false;
        }
        match space
            .tree
            .materialize(&map, fault.addr.page_align(), PAGE_SIZE)
        {
            Ok(()) => {
                return space
                    .tree
                    .translate(fault.addr)
                    .is_some_and(|(_, flags)| satisfies(flags, fault.kind));
            }
            Err(error) => {
                error!("failed to materialize page: {:?}", error);
                return false;
            }
        }
    }
    error!(
        "kernel page fault: {:?} at {:?}, pc={:#x}",
        fault.kind, fault.addr, fault.pc
    );
    false
}
