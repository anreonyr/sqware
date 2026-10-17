//! 远端 hart 那一段热窗口：清 `SIE`、装自有向量、热身、等门、采样。
//!
//! 这一整段是**测试侧**的汇编。它一进去就把 `sstatus.SIE` 清掉、把 `stvec` 换成本文件
//! 自己的向量；此后没有 ecall、没有 `satp`/`sfence.vma`、没有分配、也没有调度——
//! 于是这段窗口里不可能出现一次生产 trap（那支跳板**每次**返回都要全 `sfence.vma`，
//! 会把要证的旧 TLB 提前刷干净）。窗口里只有四件事：热身那次访存、发布 `warmed`、
//! 等 `gate`（期限只用直接 `rdtime` CSR）、采样那次访存。
//!
//! 采样那一次访存由本文件那个私有向量接住：它记下 `scause`/`stval`/`sepc`，把 `sepc`
//! 挪到恢复点上（**不重执行同一条 fault load**），`sret` 回去。出口只有恢复点一处：
//! 先把这次窗口写下的载荷推过一道完整数据栅栏、再写`status`（**唯一一处发布**），
//! 然后装回原来那支向量、再装回原来那面 `sstatus`——之后任何 ecall/中断都走回生产
//! 那一套，本段不在生产路径上留后手。
//!
//! 结论码走 **`t4`** 这个 caller-saved 寄存器：三条出口分支只写寄存器，不各自发布，
//! 于是“载荷在前、`fence rw,rw`、`status` 在后”是**结构上**成立的（而不是三处各写一遍
//! 靠人盯）。
//!
//! 每一格偏移都取自 [`Window`] 那一份 `#[repr(C)]` 布局（`offset_of!`），
//! 故这里与结构定义不会各说各话。

use core::arch::global_asm;
use core::mem::offset_of;

use super::SIE;
use super::window::Window;

global_asm!(
    ".section .text, \"ax\"",
    ".globl __mtlb_window",
    ".align 2",
    "__mtlb_window:",
    "    la    a0, {window}",
    // 进窗口第一手：先把生产向量记下，再把 `SIE` 清掉，最后才换上本文件那支。
    // 这个次序是有交代的：`SIE` 一清，S 态中断（与调度）就再也进不来了；而在它
    // 之前万一真来一发，落的是**还没换掉**的生产向量——入口还债，窗口不受影响。
    "    csrr  t0, stvec",
    "    sd    t0, {off_stvec}(a0)",
    "    li    t2, {sie}",
    "    csrrc t0, sstatus, t2",
    "    sd    t0, {off_sstatus}(a0)",
    "    la    t0, __mtlb_vector",
    "    csrw  stvec, t0",
    // 热身：本窗口对源页的第一次真实访存。
    "    ld    t1, {off_va}(a0)",
    "    la    t3, mtlb_warm",
    "    sd    t3, {off_warm_at}(a0)",
    "mtlb_warm:",
    "    lbu   t3, 0(t1)",
    "    sd    t3, {off_warm}(a0)",
    // 期限与发布：控制者看到 warmed 才动手。
    "    rdtime t6",
    "    ld    t0, {off_budget}(a0)",
    "    add   t5, t6, t0",
    "    sd    t5, {off_deadline}(a0)",
    "    fence rw, rw",
    "    li    t0, 1",
    "    sd    t0, {off_warmed}(a0)",
    // 等门：只问共享那一格与直接 rdtime，不问生产任何一处。
    "mtlb_wait:",
    "    ld    t0, {off_gate}(a0)",
    "    bnez  t0, mtlb_go",
    "    rdtime t6",
    "    bgeu  t6, t5, mtlb_late",
    "    j     mtlb_wait",
    "mtlb_late:",
    "    li    t4, {timeout}",
    "    j     mtlb_resume",
    // 采样：门开（= 移交已成功结算）之后才做的那一次访存。
    "mtlb_go:",
    "    fence rw, rw",
    "    ld    t1, {off_va}(a0)",
    "    la    t3, mtlb_sample",
    "    sd    t3, {off_sample_at}(a0)",
    "mtlb_sample:",
    "    lbu   t3, 0(t1)",
    "    sd    t3, {off_byte}(a0)",
    "    li    t4, {stale}",
    // 出口（唯一一处发布）：载荷先全部写完，再一道完整数据栅栏，最后才写 `status`。
    // 少了这一序，控制者那一侧的 Acquire 接不上一个 release 发布。
    "mtlb_resume:",
    "    fence rw, rw",
    "    sd    t4, {off_status}(a0)",
    // 先向量、后 sstatus；结论在 t4（caller-saved），`ret` 前搬到 a0。
    "    ld    t1, {off_stvec}(a0)",
    "    csrw  stvec, t1",
    "    ld    t1, {off_sstatus}(a0)",
    "    csrw  sstatus, t1",
    "    mv    a0, t4",
    "    ret",
    // 私有向量：接住那一次访存，记三格、把 sepc 挪到恢复点，原样还手。
    // 它只动 t0/t2/a0（在采样点上这三格都不是活的），并把它们按栈上的原值还回去；
    // 结论码写 t4——那是本窗口留出来的一格，`mtlb_resume` 唯一一处发布它。
    ".align 2",
    "__mtlb_vector:",
    "    addi  sp, sp, -32",
    "    sd    t0, 0(sp)",
    "    sd    t2, 8(sp)",
    "    sd    a0, 16(sp)",
    "    la    a0, {window}",
    "    csrr  t0, scause",
    "    sd    t0, {off_cause}(a0)",
    "    csrr  t0, stval",
    "    sd    t0, {off_stval}(a0)",
    "    csrr  t0, sepc",
    "    sd    t0, {off_sepc}(a0)",
    "    li    t4, {trapped}",
    "    la    t0, mtlb_resume",
    "    csrw  sepc, t0",
    "    ld    t0, 0(sp)",
    "    ld    t2, 8(sp)",
    "    ld    a0, 16(sp)",
    "    addi  sp, sp, 32",
    "    sret",
    window = sym super::window::WINDOW,
    off_va = const offset_of!(Window, va),
    off_budget = const offset_of!(Window, budget),
    off_warm = const offset_of!(Window, warm),
    off_warm_at = const offset_of!(Window, warm_at),
    off_sample_at = const offset_of!(Window, sample_at),
    off_deadline = const offset_of!(Window, deadline),
    off_sstatus = const offset_of!(Window, sstatus),
    off_stvec = const offset_of!(Window, stvec),
    off_cause = const offset_of!(Window, cause),
    off_stval = const offset_of!(Window, stval),
    off_sepc = const offset_of!(Window, sepc),
    off_byte = const offset_of!(Window, byte),
    off_warmed = const offset_of!(Window, warmed),
    off_gate = const offset_of!(Window, gate),
    off_status = const offset_of!(Window, status),
    sie = const SIE,
    timeout = const Window::TIMEOUT,
    stale = const Window::STALE,
    trapped = const Window::TRAPPED,
);

unsafe extern "C" {
    fn __mtlb_window() -> usize;
}

/// 进热窗口，出窗口时交回读者自己那一段结论码（`a0`）。
pub(super) fn enter() -> usize {
    // SAFETY: 本文件是唯一的读点；窗口只碰共享窗与它自己保存的 CSRs，
    // 且在出口把 `stvec`/`sstatus` 按原值装回。
    unsafe { __mtlb_window() }
}
