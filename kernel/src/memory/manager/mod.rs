pub mod addr;
pub mod asid;
pub mod entry;
pub mod frame;
pub mod mode;
pub mod table;
pub mod tree;

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
