//! Device/memory ordering shared by every device protocol backend.
//!
//! RISC-V separates normal R/W from I/O ordering, so device-visible buffers and
//! doorbells use `fence iorw, iorw` rather than a normal fence.

pub fn sync() {
    // SAFETY: emits one fence and touches no memory or register.
    unsafe { core::arch::asm!("fence iorw, iorw", options(nostack)) }
}
