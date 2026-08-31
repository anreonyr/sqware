// 陷入上下文（TrapContext）— trap 入口/出口保存/恢复的帧（用户线程帧 / hart 帧）
//
// 字段布局即 trap ABI：`__alltraps`/`__restore`（trampoline 汇编）按裸偏移
// 读写，一经固化不可随意增删——布局由本文件底部的编译期偏移断言锁定，与汇编
// 注释中的偏移一一对应（改布局必须先改两处）。
//
// 前 6 个字段是内核侧切换元数据：`__alltraps`保存用户上下文后读它们切回内核，
// `__restore`按它们切回目标空间。`self_pa`让内核经恒等映射
// 访问用户帧（无需重指向）；`user_satp`供 `__restore`切回目标空间页表。
// 之后是用户寄存器上下文（gpr[32] + sstatus + sepc）。
//
// 字段均以强类型承载语义（VirtAddr/PhysAddr/satp::Satp/sstatus::Sstatus/Gprs），
// 全部为单字大小、可 Copy 的零成本包装，内存布局与裸 usize 完全一致——底部偏移
// 断言不受影响；汇编侧仍按裸偏移读写，封装只约束 Rust 侧消费端（trap/envcall/
// 填帧）不把地址、状态位、寄存器号互相搞混。

use riscv::register::{satp, sstatus};

use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::work::unit::space::SpaceKind;
use crate::work::unit::team::Team;

/// 通用寄存器集（x0..x31）。`__restore`/`__alltraps` 按 `0x30 + i*8` 裸偏移存取数组。
///
/// 封装的意义在 ABI 层：用常量名替代魔法下标（a0/a7/sp...），剩余位面由
/// 内部数组承载，与汇编偏移一一对应。`Debug` 只打印非零寄存器——panic 转储
/// 时 32 个 0 无信息量。
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Gprs(pub [usize; 32]);

#[allow(unused)]
impl Gprs {
    // ── ABI 寄存器号（RISC-V calling convention：a0..a7 = x10..x17） ──
    pub const RA: usize = 1; // 返回地址
    pub const SP: usize = 2; // 栈指针
    pub const GP: usize = 3; // 全局指针
    pub const TP: usize = 4; // 线程指针（内核 = hartid）
    pub const S0: usize = 8; // 帧指针（回溯起点）
    pub const A0: usize = 10; // 环境调用参数/返回值 0
    pub const A1: usize = 11; // 环境调用参数/返回值 1
    pub const A2: usize = 12;
    pub const A3: usize = 13;
    pub const A4: usize = 14;
    pub const A5: usize = 15;
    pub const A6: usize = 16;
    pub const A7: usize = 17; // 环境调用号（a7）

    /// 读取寄存器 x_i。
    #[inline]
    pub fn x(&self, i: usize) -> usize {
        self.0[i]
    }

    /// 写入寄存器 x_i（x0 恒 0，写入即 bug——debug 断言兜住）。
    #[inline]
    pub fn set_x(&mut self, i: usize, v: usize) {
        debug_assert!(i != 0, "x0 恒 0，不可写");
        self.0[i] = v;
    }
}

impl core::fmt::Debug for Gprs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "gpr{{")?;
        let mut any = false;
        for (i, v) in self.0.iter().enumerate().skip(1) {
            if *v != 0 {
                if any {
                    write!(f, " ")?;
                }
                write!(f, "x{i}={v:#x}")?;
                any = true;
            }
        }
        write!(f, "}}")
    }
}

/// 陷入上下文帧 — 存于帧页（用户线程帧 = Frame 窗口分配页；hart 帧 = 帧区页）。
#[derive(Debug)]
#[repr(C)]
pub struct TrapContext {
    /// 切入用户态前的内核 satp token（`__alltraps`切回内核用）。
    pub kernel_satp: satp::Satp,
    /// 切入用户态前的内核栈指针（hart 帧 = per-hart trap 栈顶；用户帧 = 任务内核栈顶）。
    pub kernel_sp: VirtAddr,
    /// 陷阱处理入口地址（内核镜像链接地址，`jalr`目标）。
    pub trap_handler: VirtAddr,
    /// trap 栈损坏标记（内核 trap 栈溢出检测：与栈底 canary 比对）。
    pub trap_stack_corrupt: usize,
    /// 本帧自身物理地址。
    pub user_pa: PhysAddr,
    /// 本空间 satp token（`__restore`切回目标空间用）。
    pub user_satp: satp::Satp,
    /// 被中断上下文通用寄存器（x0 恒 0 不存；x2=sp 在 gpr[2]）。
    pub gpr: Gprs,
    /// 被中断的 sstatus（SPP/SPIE 由 sret 消费）。
    pub sstatus: sstatus::Sstatus,
    /// 被中断的 sepc（sret 返回地址）。
    pub sepc: VirtAddr,
    /// 本帧在目标空间中的虚拟地址（restore 切表后经此 VA 收尾）。
    ///
    /// 用户线程帧 = 本空间 Frame 窗口分配的 VA；
    /// hart 帧 = 帧区页。alltraps 用户路径把
    /// sscratch 设为该 VA，使每线程帧可位于任意页而汇编零改动。
    /// （sscratch 约定：用户态 = 线程帧 self_va；内核态 = 本 hart 帧 VA，
    /// 见 trampoline 模块头。）
    pub self_va: VirtAddr,
}

impl TrapContext {
    /// 初始化为新任务入口上下文：内核切换元数据自 per-hart 帧模板拷入；
    /// 用户上下文 = 入口/栈顶/a0。SPP 与 user_satp 均自团队空间派生——任务模式 =
    /// 空间 kind（单一事实源），satp 位布局（模式|asid|root）为帧初始化职责，调用方
    /// 不必知道。SPIE=1（sret 后 SIE=1，可被抢占）。kernel_sp 不在此写——`prepare`
    /// 对每次上台无条件重写（含 steal 迁移）。
    ///
    /// # Safety
    /// 调用方须持有对 `self` 所指帧的唯一可写引用（新任务帧未发布、S-only 映射）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn init(
        &mut self,
        template: &TrapContext,
        team: &Team,
        entry: VirtAddr,
        stack_top: VirtAddr,
        arg: usize,
        pa: PhysAddr,
        self_va: VirtAddr,
    ) {
        self.kernel_satp = template.kernel_satp;
        self.trap_handler = template.trap_handler;
        self.trap_stack_corrupt = template.trap_stack_corrupt;
        self.user_pa = pa;
        // user_satp = 模式位 << 60 | asid << 44 | root_ppn —— 切回本空间用；
        // 模式位随探测所得 mode()（Sv39=8/Sv48=9/Sv57=10），非硬编码。
        self.user_satp = satp::Satp::from_bits(
            (crate::memory::manager::mode::mode().into_usize() << 60)
                | (team.space.asid() << 44)
                | team.space.root(),
        );
        self.self_va = self_va;
        self.sepc = entry;
        self.gpr.set_x(Gprs::SP, stack_top.as_usize());
        self.gpr.set_x(Gprs::A0, arg);
        // 全零起步：不继承内核当前 sstatus 的 FS/XS 等位（`__restore` 整字 csrw）。
        let mut ss = sstatus::Sstatus::from_bits(0);
        ss.set_spie(true);
        ss.set_spp(if matches!(team.space.kind(), SpaceKind::Kernel) {
            sstatus::SPP::Supervisor
        } else {
            sstatus::SPP::User
        });
        self.sstatus = ss;
    }
}

// 布局即 ABI：偏移断言与 trampoline 汇编硬编码偏移一一对应。
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
