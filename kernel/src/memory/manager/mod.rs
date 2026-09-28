pub mod addr;
pub mod asid;
pub mod entry;
pub mod fault;
pub mod mode;
pub mod table;

pub use table::MapError;

use asid::Asid;

#[inline(always)]
pub unsafe fn flush_asid(asid: Asid) {
    unsafe {
        core::arch::asm!("sfence.vma zero, {}", in(reg) asid.get());
    }
}