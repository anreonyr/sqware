#![no_std]
#![no_main]

//! 内核的**入口那一半**——只剩 `_start` 与 `main`。
//!
//! **照实记（用户裁定"迁移到 embedded-test"）**：模块树搬去了 [`kernel`]（`src/lib.rs`），
//! 本文件只留入口。这一份 `_start` 与 `tests/embedded.rs` 那一份**同形**，差别只有一行：
//! 测试目标多存一个 `dtp`（`embedded-test` 的入口不收参数），且它的 `j main` 落进
//! **embedded-test 导出的** `main`。
//!
//! 两者**不会同框**：`main` 由本文件定义、而测试目标不链 bin（`[[bin]] test = false`），
//! 故不存在符号冲突。

use core::arch::global_asm;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    la   t0, PER_HART", // &PER_HART[0]（恒等映射，Bare 下 PC 相对即物理地址）
    "    slli t1, a0, 6",    // hartid · 64（PerHart 槽宽 2⁶，编译期断言锁死）
    "    add  tp, t0, t1",   // tp = 本 hart PerHart 指针（入口约定，见 `hart_id()`）
    "    csrc sstatus, 2",
    "    la   sp, _kernel_edge",
    "    ld   t0, _canary",
    "    sd   t0, 0(sp)",
    "    la   t0, _stack",
    "    ld   t0, 0(t0)",
    "    add  sp, sp, t0",
    "    j    main",
);

#[unsafe(no_mangle)]
extern "C" fn main(hartid: usize, dtp: usize) -> ! {
    kernel::main(hartid, dtp)
}
