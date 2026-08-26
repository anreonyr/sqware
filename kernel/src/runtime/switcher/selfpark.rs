// ktask 自切换原语 — 内核闭包任务（ktask）体内主动 park/唤醒续跑
//
// 背景：`scheduler::park` 只服务 trap 上下文（envcall Sleep 分支）——被中断现场
// 已由 __alltraps/persist_kernel_preempt 落入任务帧，park 只做簿记 + 取下一帧，
// 调用方 restore。闭包体（ktask_entry 内执行的 Rust 闭包）主动 park 时，现场在
// 任务自己的内核栈上、不在帧里——直接调 scheduler::park 会丢现场：唤醒后 sret
// 回旧断点（入口/上次抢占点），不是 park 调用点之后的代码。
//
// 本原语（`ktask_park`）是 trap 路径 park 的闭包体侧对称物：
//
//   1. SIE=0 关中断（捕获现场期间不得被 S-timer 打断——persist_kernel_preempt
//      会把被打断现场拷入任务帧，覆写我们正在写的捕获现场）
//   2. 捕获当前现场（gpr/sstatus/sepc）写入本任务帧；sepc 指向 resume 标签
//   3. 簿记（调度器 park：Running → Blocked + tock 登记）→ 取下一帧 PA
//   4. restore(next)：切走（返回下一任务帧，__restore 消费）
//   5. 唤醒：unpark 从 blocked 摘除 → Starved 入队 → 上台 prepare（写
//      kernel_sp/arm timer）→ restore(本任务帧) → sret 到 sepc = resume 标签
//   6. resume：开中断（sret 后 SIE 由恢复的 sstatus 决定，此处显式置位）→
//      ret 回闭包体内调用点之后（ra 已在捕获时保存）→ 续跑 poll 循环
//
// 簿记期间无锁冲突：running 恒为当前任务；汇编只用 t 寄存器做临时（caller-
// saved，闭包体不依赖其跨调用存活），ra/sp 原样保留在帧中。

use core::arch::global_asm;

use crate::runtime::switcher::trampoline;

global_asm!(
    ".section .text",
    ".balign 4",
    ".globl ktask_park_self",
    "ktask_park_self:",
    // 入参：a0 = 睡眠毫秒（u64），a1 = 本任务帧 PA（恒等映射可写）
    // 前提：调用点现场 = 闭包体栈（任务内核栈）上。
    //
    // 0. 关中断：捕获现场与簿记期间不得被 S-timer 打断（persist_kernel_preempt
    //    会把被打断现场拷入任务帧，覆写我们正在写入的捕获现场）。
    "    csrc sstatus, 2",
    //
    // 1. 捕获现场 → 任务帧（a1 寻址，不改 a1；t0 此时仍是原始值）
    "    sd   x1,  0x38(a1)",   // gpr[1] = ra（闭包体调用点返回地址）
    "    sd   x2,  0x40(a1)",   // gpr[2] = sp（闭包体栈指针）
    "    sd   x3,  0x48(a1)",   // gp
    "    sd   x4,  0x50(a1)",   // tp（= hartid，保持）
    "    sd   x5,  0x58(a1)",
    "    sd   x6,  0x60(a1)",
    "    sd   x7,  0x68(a1)",
    "    sd   x8,  0x70(a1)",
    "    sd   x9,  0x78(a1)",
    "    sd   x10, 0x80(a1)",
    "    sd   x11, 0x88(a1)",
    "    sd   x12, 0x90(a1)",
    "    sd   x13, 0x98(a1)",
    "    sd   x14, 0xa0(a1)",
    "    sd   x15, 0xa8(a1)",
    "    sd   x16, 0xb0(a1)",
    "    sd   x17, 0xb8(a1)",
    "    sd   x18, 0xc0(a1)",
    "    sd   x19, 0xc8(a1)",
    "    sd   x20, 0xd0(a1)",
    "    sd   x21, 0xd8(a1)",
    "    sd   x22, 0xe0(a1)",
    "    sd   x23, 0xe8(a1)",
    "    sd   x24, 0xf0(a1)",
    "    sd   x25, 0xf8(a1)",
    "    sd   x26, 0x100(a1)",
    "    sd   x27, 0x108(a1)",
    "    sd   x28, 0x110(a1)",
    "    sd   x29, 0x118(a1)",
    "    sd   x30, 0x120(a1)",
    "    sd   x31, 0x128(a1)",
    // sstatus：显式化 SPP=S（bit8=1）与 SPIE=1（bit5）——sret 用 SPIE 复位 SIE，
    // 唤醒后须可被抢占；不清其它位（SUM/XS 等按现值保留）
    "    csrr t0, sstatus",
    "    ori  t0, t0, (1 << 5)",        // SPIE = 1
    "    li   t1, 3",
    "    slli t1, t1, 8",
    "    not  t1, t1",
    "    and  t0, t0, t1",              // 清 SPP 两位
    "    ori  t0, t0, (1 << 8)",        // SPP = S
    "    sd   t0, 0x130(a1)",           // frame.sstatus
    "    la   t0, 2f",
    "    sd   t0, 0x138(a1)",           // frame.sepc = resume 标签（链接地址，
    //                                    // 唤醒后恢复的 satp = 任务 user_satp =
    //                                    // 内核空间表，链接地址可访问）
    //
    // 2. 簿记：调度器 park（Running → Blocked + tock + 下一帧 PA）。ra 不动
    //    （闭包体返回地址已在帧中）；寄存器传参 a0 = 毫秒。返回 a0 = 下一帧 PA。
    "    la   t1, ktask_park_ms",
    "    jalr ra, t1",
    // 3. 切走：restore(下一帧 PA)。a0 已是下一帧 PA；restore 不返回。
    "    la   t1, ktask_restore",
    "    jalr t1, t1",
    // 4. resume 标签：唤醒后 sret 到这里（__restore 已恢复 gpr/sstatus/sepc，
    //    执行环境 = 捕获时闭包体现场，帧 = 任务帧）
    "    2:",
    "    csrsi sstatus, 2",             // SIE = 1（sret 后显式开中断，续跑可抢占）
    "    ret",                          // ra = 捕获时保存的闭包体调用点 → 续跑
    ".globl ktask_park_self_end",
    "ktask_park_self_end:",
);

// ── 簿记 / 恢复的外部 C 桥（global_asm 按符号名 la，须 no_mangle）──

/// 调度器 park 簿记：当前任务 Running → Blocked(Park)，登记 tock，取下一帧 PA。
/// # Safety
/// 仅由 `ktask_park_self` 汇编调用：running 恒为当前任务；现场已在帧中。
// SAFETY: 纯粹另册函数桥，无 unsafe 操作；外部不可见。
#[unsafe(no_mangle)]
unsafe extern "C" fn ktask_park_ms(ms: u64) -> usize {
    crate::work::room::scheduler::park_ktask(ms)
}

/// 恢复目标帧（恒等映射 PA）并进入其中。永不返回。
/// # Safety
/// 仅由 `ktask_park_self` 汇编调用；`frame_pa` 必须是有效任务帧。
// SAFETY: 另册桥，转发 trampoline::restore（本就要求有效帧 PA）。
#[unsafe(no_mangle)]
unsafe extern "C" fn ktask_restore(frame_pa: usize) -> ! {
    trampoline::restore(frame_pa)
}

/// 闭包体内主动 park（ktask 自切换原语的 Rust 面）。
///
/// 语义：把当前内核任务挂起到 `duration`（tick 粒度，到点经 tock 唤醒），
/// 唤醒后从本调用点之后继续（续跑 poll 循环）。与 `scheduler::park`（trap 路径）
/// 的区别：本函数捕获闭包体内现场并自切换，不依赖 trap 上下文。
pub fn ktask_park(duration: core::time::Duration) {
    let ms = duration.as_millis() as u64;
    // 当前任务帧 PA（锁内取、放锁返回；闭包体运行期间 running 恒为本任务）
    let pa = crate::work::room::scheduler::running_task_pa();
    // SAFETY: pa 为本任务专属帧（恒等映射可变）；闭包体场景 running 恒在。
    //
    // `call` 调 ktask_park_self：call 令 ra = 本 asm 之后（ktask_park 函数尾），
    // 汇编捕获该 ra；唤醒后 resume 标签 ret 回该处 → ktask_park 正常返回闭包体。
    // clobber_abi("C")：汇编内部（ktask_park_self 全程）破坏全部 caller-saved
    // 寄存器——唤醒恢复路径由帧还原现场，Rust 侧不依赖任何 caller-saved 值。
    unsafe {
        core::arch::asm!(
            "call ktask_park_self",
            in("a0") ms,
            in("a1") pa,
            clobber_abi("C"),
        );
    }
}