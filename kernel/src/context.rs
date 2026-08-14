// 陷入上下文（TrapContext）— trap 入口/出口在 TRAP_CONTEXT 页保存/恢复的帧
//
// 本阶段仅 memory 子系统引用（用户空间构建 `space::Space::build` 把内核切换
// 元数据拷进任务帧、`space::init` 经恒等映射访问），trap 汇编尚未接入。字段布局
// 一经 trap 子系统与汇编固化后不可随意增删。地址字段用 PhysAddr/VirtAddr 新类型
// （均 #[repr(transparent)]，与 usize 同布局同 ABI），固化前后类型均可自由调整。

use crate::memory::manager::addr::{PhysAddr, VirtAddr};

/// 陷入上下文帧 — 存于每空间独占的 TRAP_CONTEXT 页（S 态独占、无 U 位）。
///
/// 前 5 个字段是内核侧切换元数据：`__alltraps` 保存用户上下文后读它们切回内核，
/// `__restore` 按它们切回目标空间。`self_pa` 让内核在 KERNEL_SPACE 经恒等映射
/// 访问用户帧（无需重指向）。用户寄存器上下文待 trap 汇编接入后追加。
#[repr(C)]
pub struct TrapContext {
    /// 切入用户态前的内核 satp token（`__alltraps` 切回内核用）。
    pub kernel_satp: usize,
    /// 切入用户态前的内核栈指针。
    pub kernel_sp: VirtAddr,
    /// 陷阱处理入口地址（stvec 目标）。
    pub trap_handler: VirtAddr,
    /// trap 栈损坏标记（内核 trap 栈溢出检测）。
    pub trap_stack_corrupt: usize,
    /// 本帧自身物理地址。
    pub self_pa: PhysAddr,
}
