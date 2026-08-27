// ── 适配层：ktask ──
//
// 内核任务面——与用户任务同帧 ABI 的自愿切换服务（软陷阱）。
// 每个服务 = naked 头 + 既有调度核心 + restore 尾：
//   park/starve：头把调用点上下文存进 hart 帧（csrrw 交换取帧址；先存原 t0/t1
//     再用 scratch，与 __strap 同序），桥接 persist 搬进任务帧，经调度核心取
//     下一帧后 restore——唤醒时 __restore sret 回调用点，闭包视角下服务调用
//     像普通函数返回。
//   reap：终端服务，无恢复点——头只做关中断 + 切 per-hart trap 栈（clear 契约：
//     回收不在被回收的任务栈上执行），再走 reap 核心。
// 命名：三个服务与核心/用户面同词（park/starve/reap），路径 + 签名区分——
//   `Conductor::park`(核心方法) / `utask::starve`(用户面) / 本模块(内核面)。

use core::arch::naked_asm;
use core::time::Duration;

use crate::runtime::switcher::trap::{persist_kernel_preempt, trap_stack_edge};
use crate::runtime::switcher::trampoline::restore;

use super::utask::{park as sched_park, reap as sched_reap, starve as sched_starve};

/// 内核任务睡眠：存帧 → park 核心 → 切走；唤醒后恢复于调用点。
///
/// # 行为约定（返回两次）
/// 本次调用沿 naked 头直接 `restore` 切走，**这次不返回**；唤醒后经帧恢复
/// 从调用点「第二次返回」——故签名声明为普通返回（`()`），编译器才会保留
/// 调用点之后的代码作为恢复点（若声明 `-> !`，后续代码被折叠，见历史）。
///
/// # Safety
/// 仅可由当前 running 的内核任务（S 态任务上下文）调用——其余上下文调用是
/// 设计错误（帧交换依赖内核态 sscratch 约定，服务只在任务上下文中有效）。
// Duration 经 extern 边界按值传两寄存器（naked 头原样保留下传；不跨真实 FFI）。
#[allow(improper_ctypes_definitions)]
#[unsafe(naked)]
pub extern "C" fn park(_duration: Duration) {
    naked_asm!(
        // 关 SIE：hart 帧作 scratch 期间免嵌套陷阱覆写
        "csrc sstatus, 2",
        // sp ← 本 hart 帧 VA（内核态 sscratch 约定）；sscratch ← 闭包 sp
        "csrrw sp, sscratch, sp",
        // 全通用寄存器原值入帧（先存后用；交换只动 sp，t0/t1 未破坏）
        "sd    x1,  0x38(sp)",
        "sd    x3,  0x48(sp)",
        "sd    x4,  0x50(sp)",
        "sd    x5,  0x58(sp)",
        "sd    x6,  0x60(sp)",
        "sd    x7,  0x68(sp)",
        "sd    x8,  0x70(sp)",
        "sd    x9,  0x78(sp)",
        "sd    x10, 0x80(sp)",
        "sd    x11, 0x88(sp)",
        "sd    x12, 0x90(sp)",
        "sd    x13, 0x98(sp)",
        "sd    x14, 0xa0(sp)",
        "sd    x15, 0xa8(sp)",
        "sd    x16, 0xb0(sp)",
        "sd    x17, 0xb8(sp)",
        "sd    x18, 0xc0(sp)",
        "sd    x19, 0xc8(sp)",
        "sd    x20, 0xd0(sp)",
        "sd    x21, 0xd8(sp)",
        "sd    x22, 0xe0(sp)",
        "sd    x23, 0xe8(sp)",
        "sd    x24, 0xf0(sp)",
        "sd    x25, 0xf8(sp)",
        "sd    x26, 0x100(sp)",
        "sd    x27, 0x108(sp)",
        "sd    x28, 0x110(sp)",
        "sd    x29, 0x118(sp)",
        "sd    x30, 0x120(sp)",
        "sd    x31, 0x128(sp)",
        // 至此全部原值已入帧，scratch 自由；时长暂存 s-regs（随 ABI 穿越 persist）
        "mv    s0, a0",
        "mv    s1, a1",
        "csrr  t0, sscratch",
        "sd    t0,  0x40(sp)",              // gpr[2] = 闭包 sp
        "csrr  t0, sstatus",
        "andi  t0, t0, -3",                 // 清 SIE
        "ori   t0, t0, (1 << 5) | (1 << 8)",// SPIE=1、SPP=1（唤醒恢复内核态、可再抢占）
        "sd    t0,  0x130(sp)",
        "sd    ra,  0x138(sp)",             // sepc = 调用点返回地址
        "mv    a0, sp",                     // persist 入参 = 本 hart 帧
        "ld    sp,  0x08(sp)",              // 切 per-hart trap 栈（同 __strap）
        "la    t0, {persist}",              // hart 帧 → running 任务帧
        "jalr  t0",
        "mv    a0, s0",                     // 时长（persist 按 ABI 保全 s-regs）
        "mv    a1, s1",
        "la    t0, {sched_park}",
        "jalr  t0",                          // a0 = 下一帧 PA
        "la    t0, {restore}",
        "jalr  t0",                          // 永不返回
        persist = sym persist_kernel_preempt,
        sched_park = sym sched_park,
        restore = sym restore,
    );
}

/// 内核任务让出：存帧 → starve 核心 → 切走（同 park，无时长参数）。
///
/// # 行为约定
/// 返回两次（同 [`park`](Self::park)）：本次调用不返回，轮转后从调用点恢复。
///
/// # Safety
/// 仅可由当前 running 的内核任务调用（同 [`park`](Self::park)）。
// 预留：内核闭包主动让出（当前演示集未使用；API 面与 park/reap 同族）。
#[allow(dead_code)]
#[unsafe(naked)]
pub extern "C" fn starve() {
    naked_asm!(
        "csrc sstatus, 2",
        "csrrw sp, sscratch, sp",
        "sd    x1,  0x38(sp)",
        "sd    x3,  0x48(sp)",
        "sd    x4,  0x50(sp)",
        "sd    x5,  0x58(sp)",
        "sd    x6,  0x60(sp)",
        "sd    x7,  0x68(sp)",
        "sd    x8,  0x70(sp)",
        "sd    x9,  0x78(sp)",
        "sd    x10, 0x80(sp)",
        "sd    x11, 0x88(sp)",
        "sd    x12, 0x90(sp)",
        "sd    x13, 0x98(sp)",
        "sd    x14, 0xa0(sp)",
        "sd    x15, 0xa8(sp)",
        "sd    x16, 0xb0(sp)",
        "sd    x17, 0xb8(sp)",
        "sd    x18, 0xc0(sp)",
        "sd    x19, 0xc8(sp)",
        "sd    x20, 0xd0(sp)",
        "sd    x21, 0xd8(sp)",
        "sd    x22, 0xe0(sp)",
        "sd    x23, 0xe8(sp)",
        "sd    x24, 0xf0(sp)",
        "sd    x25, 0xf8(sp)",
        "sd    x26, 0x100(sp)",
        "sd    x27, 0x108(sp)",
        "sd    x28, 0x110(sp)",
        "sd    x29, 0x118(sp)",
        "sd    x30, 0x120(sp)",
        "sd    x31, 0x128(sp)",
        "csrr  t0, sscratch",
        "sd    t0,  0x40(sp)",
        "csrr  t0, sstatus",
        "andi  t0, t0, -3",
        "ori   t0, t0, (1 << 5) | (1 << 8)",
        "sd    t0,  0x130(sp)",
        "sd    ra,  0x138(sp)",
        "mv    a0, sp",
        "ld    sp,  0x08(sp)",
        "la    t0, {persist}",
        "jalr  t0",
        "la    t0, {sched_starve}",
        "jalr  t0",
        "la    t0, {restore}",
        "jalr  t0",
        persist = sym persist_kernel_preempt,
        sched_starve = sym sched_starve,
        restore = sym restore,
    );
}

/// 内核任务退出（终端服务，无恢复点）：关中断 → 切 per-hart trap 栈 →
/// reap 核心 + clear + run → 切走。`ktask_exit`/`kstack_exit` 消解于此。
///
/// # Safety
/// 仅可由当前 running 的内核任务调用（同 [`park`](Self::park)）。
#[unsafe(naked)]
pub extern "C" fn reap() -> ! {
    naked_asm!(
        "csrc sstatus, 2",                  // 退出序列原子
        "mv    a0, tp",                     // hart id → 栈顶查询
        "la    t0, {edge}",
        "jalr  t0",                          // a0 = 本 hart trap 栈顶 VA
        "mv    sp, a0",                      // clear() 契约：回收不在任务栈上执行
        "la    t0, {sched_reap}",
        "jalr  t0",                          // a0 = 下一帧 PA
        "la    t0, {restore}",
        "jalr  t0",
        edge = sym trap_stack_edge,
        sched_reap = sym sched_reap,
        restore = sym restore,
    );
}