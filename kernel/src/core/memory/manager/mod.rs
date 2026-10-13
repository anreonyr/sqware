pub mod addr;
pub mod asid;
pub mod entry;
pub mod fault;
pub mod frame;
pub mod mode;
pub mod table;
pub mod tree;

pub use fault::FaultKind;
pub use frame::Frame;
pub use table::MapError;
pub use tree::{BorrowedRegion, Map, Tree};

use asid::Asid;

#[inline(always)]
pub unsafe fn flush_asid(asid: Asid) {
    unsafe {
        if asid.is_kernel() {
            core::arch::asm!("sfence.vma zero, zero");
        } else {
            core::arch::asm!("sfence.vma zero, {}", in(reg) asid.get());
        }
    }
}

pub fn sync_instructions() -> Result<(), MapError> {
    unsafe { core::arch::asm!("fence rw, rw", options(nostack)) };
    crate::machine::hart::request_instruction_sync();
    unsafe { core::arch::asm!("fence rw, rw", "fence.i", options(nostack)) };
    let me = crate::machine::hart::hart_id();
    let mut mask = 0usize;
    for h in (0..crate::machine::hart::hart_count()).map(crate::machine::hart::HartId::new) {
        if h != me && asid::lease(h).is_some() {
            mask |= h.bit().1;
        }
    }
    sbi::RfenceCall::new(sbi::fid::Rfence::RemoteFenceI)
        .args(sbi::ecall::SArgs {
            a0: mask,
            a1: 0,
            ..Default::default()
        })
        .call()
        .map(|_| ())
        .map_err(|_| MapError::NoRegion)
}
