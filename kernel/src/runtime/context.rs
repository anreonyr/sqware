// 陷入上下文（TrapContext）— trap 入口/出口在 TRAP_CONTEXT 页保存/恢复的帧
//
// 字段布局即 trap ABI：`__alltraps`/`__restore`（runtime::trampoline 汇编）按裸偏移
// 读写，一经固化不可随意增删——布局由本文件底部的编译期偏移断言锁定，与汇编
// 注释中的偏移一一对应（改布局必须先改两处）。
//
// 前 6 个字段是内核侧切换元数据：`__alltraps`保存用户上下文后读它们切回内核，
// `__restore`按它们切回目标空间。`self_pa`让内核在 KERNEL_SPACE 经恒等映射
// 访问用户帧（无需重指向）；`user_satp`供 `__restore`切回目标空间页表。
// 之后是用户寄存器上下文（gpr[32] + sstatus + sepc）。

use crate::memory::manager::addr::{PhysAddr, VirtAddr};

/// 陷入上下文帧 — 存于每空间独占的 TRAP_CONTEXT 页（S 态独占、无 U 位）。
#[repr(C)]
pub struct TrapContext {
    /// 切入用户态前的内核 satp token（`__alltraps`切回内核用）。
    pub kernel_satp: usize,
    /// 切入用户态前的内核栈指针（内核帧 = per-hart trap 栈顶；用户帧 = 任务内核栈顶）。
    pub kernel_sp: VirtAddr,
    /// 陷阱处理入口地址（内核镜像链接地址，`jalr`目标）。
    pub trap_handler: VirtAddr,
    /// trap 栈损坏标记（内核 trap 栈溢出检测：与栈底 canary 比对）。
    pub trap_stack_corrupt: usize,
    /// 本帧自身物理地址。
    pub user_pa: PhysAddr,
    /// 本空间 satp token（`__restore`切回目标空间用）。
    pub user_satp: usize,
    /// 被中断上下文通用寄存器（x0 恒 0 不存；x2=sp 在 gpr[2]）。
    pub gpr: [usize; 32],
    /// 被中断的 sstatus（SPP/SPIE 由 sret 消费）。
    pub sstatus: usize,
    /// 被中断的 sepc（sret 返回地址）。
    pub sepc: usize,
    /// 本帧在目标空间中的虚拟地址（restore 切表后经此 VA 收尾）。
    ///
    /// 用户线程帧 = 本空间 Frame 窗口分配的 VA（不再固定 TRAP_CONTEXT）；
    /// 内核帧 = per-hart 帧（hart 0 即 TRAP_CONTEXT）。alltraps 用户路径把
    /// sscratch 设为该 VA，使每线程帧可位于任意页而汇编零改动。
    pub self_va: usize,
}

// 布局即 ABI：偏移断言与 runtime/trampoline.rs 汇编硬编码偏移一一对应。
const _: () = {
    assert!(core::mem::offset_of!(TrapContext, kernel_satp) == 0x00);
    assert!(core::mem::offset_of!(TrapContext, kernel_sp) == 0x08);
    assert!(core::mem::offset_of!(TrapContext, trap_handler) == 0x10);
    assert!(core::mem::offset_of!(TrapContext, trap_stack_corrupt) == 0x18);
    assert!(core::mem::offset_of!(TrapContext, user_pa) == 0x20);
    assert!(core::mem::offset_of!(TrapContext, user_satp) == 0x28);
    assert!(core::mem::offset_of!(TrapContext, gpr) == 0x30);
    assert!(core::mem::offset_of!(TrapContext, sstatus) == 0x130);
    assert!(core::mem::offset_of!(TrapContext, sepc) == 0x138);
    assert!(core::mem::offset_of!(TrapContext, self_va) == 0x140);
};
