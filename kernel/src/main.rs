#![no_std]
#![no_main]
#![feature(allocator_api)]

// mod lock;
mod panicking;

use core::arch::{asm, global_asm};

global_asm!(
    ".section .text._start",
    ".globl _early_stack_top",
    ".globl _start",
    "_start:",
    "    la   sp, _early_stack_top",
    "    j    main",
);

#[unsafe(no_mangle)]
extern "C" fn main() {
    loop {
        unsafe {
            asm!("wfi");
        }
    }
}
