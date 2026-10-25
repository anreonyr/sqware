//! The single program startup path: arguments, TLS bootstrap and clean return.

#[cfg(target_arch = "riscv64")]
use core::arch::global_asm;

#[cfg(target_arch = "riscv64")]
global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call save_args",
    "    call tls_bootstrap",
    "    addi sp, sp, -8",
    "    mv   a0, sp",
    "    call clean_ret",
    "1: j 1b",
);

#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { crate::boot::bootstrap() }
}
