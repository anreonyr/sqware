use riscv::register::{scause, sepc, stval};

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::FaultKind;

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
