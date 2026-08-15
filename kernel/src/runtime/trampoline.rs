// 陷阱 trampoline — 所有地址空间共同映射、共同取指的 trap 入口页
//
// 一页（4 KiB）内含 `__alltraps`（保存帧 + 切 satp）与 `__restore`（切回 + 恢复 + sret），
// 内核空间与所有用户空间以 TRAMPOLINE VA 映射同一物理页（G 位，见
// manager::space::Space::TRAMPOLINE），`stvec` 指向 `__alltraps`。
//
// 本页代码执行于 TRAMPOLINE 固定 VA（0xFFFF_FFFF_FFFF_F000）——任何 PC 相对寻址
// （la/call 等）的目标必须在本页内；跨页符号（如 _trap_stack_top、Rust 的
// trap_handler）只能经帧内元数据（kernel_sp / trap_handler 字段）或绝对常量（LUI）
// 寻址。本页代码无 PC 相对跨页引用，故在链接地址（0x8020_0000+）与 TRAMPOLINE VA
// 两处取指均正确。TRAP_CONTEXT 的 LUI 立即数由 Rust 常量 TRAP_CONTEXT_LUI 注入
// （单一来源，改 space::TRAP_CONTEXT 即可）。
//
// sscratch 约定：用户态 = 当前线程帧 VA（帧内 self_va 字段，帧窗口分配，不再
// 固定 TRAP_CONTEXT）；内核态 = 0。`__restore` 按恢复的 sstatus.SPP 复原该约定
// （SPP=0 从帧内 self_va 字段读取——每线程帧位置可任意，`__alltraps` 零改动）。
//
// 帧布局与偏移见 runtime/context.rs（编译期偏移断言锁定，改布局必须先改两处）。

use core::arch::global_asm;

use crate::memory::manager::addr::PhysAddr;
use crate::memory::manager::space::{TRAMPOLINE, TRAP_CONTEXT};

/// TRAP_CONTEXT 的 LUI 立即数（bits[31:12]）——汇编经 `const` 注入，单一来源。
///
/// LUI 把 20 位立即数符号扩展后左移 12 位，与 `VirtAddr::from_raw` 的符号扩展
/// 语义一致；改 `space::TRAP_CONTEXT` 即可，勿手改汇编。
const TRAP_CONTEXT_LUI: usize = (TRAP_CONTEXT.as_usize() >> 12) & 0xFFFFF;

// 编码断言：LUI 立即数符号扩展必须能还原 TRAP_CONTEXT（VA 不再满足 LUI 编码时编译期报错）。
const _: () = {
    let shift = usize::BITS as usize - 20;
    let imm = ((TRAP_CONTEXT_LUI as isize) << shift) >> shift;
    assert!(((imm as usize) << 12) == TRAP_CONTEXT.as_usize());
};

global_asm!(
    ".section .trampoline, \"ax\"",
    ".align 12",
    ".globl __trampoline_start",
    "__trampoline_start:",

    // ── 陷阱入口（stvec Direct 目标）──────────────────────────────────
    ".globl __alltraps",
    "__alltraps:",
    "    csrr  t0, sstatus",
    "    andi  t0, t0, (1 << 8)",          // SPP：0 = 来自用户态，1 = 来自内核态
    "    bnez  t0, __strap",

    // ── 用户态陷阱（__utrap）：现场存本空间 TRAP_CONTEXT 帧 ────────────
    "__utrap:",
    "    csrrw sp, sscratch, sp",          // sp = TRAP_CONTEXT VA；sscratch = 用户 sp
    "    sd    x1,  0x38(sp)",             // gpr[1] = ra
    "    csrr  t0, sscratch",
    "    sd    t0,  0x40(sp)",             // gpr[2] = 用户 sp
    "    sd    x3,  0x48(sp)",
    "    sd    x4,  0x50(sp)",
    "    sd    x5,  0x58(sp)",
    "    sd    x6,  0x60(sp)",
    "    sd    x7,  0x68(sp)",
    "    sd    x8,  0x70(sp)",
    "    sd    x9,  0x78(sp)",
    "    sd    x10, 0x80(sp)",
    "    sd    x11, 0x88(sp)",
    "    sd    x12, 0x90(sp)",
    "    sd    x13, 0x98(sp)",
    "    sd    x14, 0xa0(sp)",
    "    sd    x15, 0xa8(sp)",
    "    sd    x16, 0xb0(sp)",
    "    sd    x17, 0xb8(sp)",
    "    sd    x18, 0xc0(sp)",
    "    sd    x19, 0xc8(sp)",
    "    sd    x20, 0xd0(sp)",
    "    sd    x21, 0xd8(sp)",
    "    sd    x22, 0xe0(sp)",
    "    sd    x23, 0xe8(sp)",
    "    sd    x24, 0xf0(sp)",
    "    sd    x25, 0xf8(sp)",
    "    sd    x26, 0x100(sp)",
    "    sd    x27, 0x108(sp)",
    "    sd    x28, 0x110(sp)",
    "    sd    x29, 0x118(sp)",
    "    sd    x30, 0x120(sp)",
    "    sd    x31, 0x128(sp)",
    "    csrr  t0, sstatus",
    "    sd    t0,  0x130(sp)",            // sstatus
    "    csrr  t0, sepc",
    "    sd    t0,  0x138(sp)",            // sepc
    // 读内核切换元数据（趁 sp 仍是用户帧、用户 satp 有效）
    "    ld    t0,  0x00(sp)",             // kernel_satp
    "    ld    t1,  0x08(sp)",             // kernel_sp
    "    ld    t2,  0x10(sp)",             // trap_handler
    "    ld    a0,  0x20(sp)",             // self_pa
    // 切内核页表（PC 仍在 trampoline，两空间同 VA，安全）
    "    csrw  satp, t0",
    "    sfence.vma",
    // 切内核栈；旧 sp（TRAP_CONTEXT VA）切表后指向内核帧，不再解引用
    "    mv    sp, t1",
    "    jalr  t2",                        // handler(frame_pa) -> frame_pa（阶段 A 恒原帧）
    "    j     __restore",

    // ── 内核态陷阱（__strap）：现场存内核帧（TRAP_CONTEXT VA），栈切 per-hart trap 栈 ──
    "__strap:",
    "    csrrw sp, sscratch, sp",          // sp = 0（内核态约定）；sscratch = 被中断内核 sp
    "    lui   sp, {tc}",                 // sp = TRAP_CONTEXT VA（内核帧，内核空间映射）
    "    sd    x1,  0x38(sp)",
    "    sd    x3,  0x48(sp)",
    "    sd    x4,  0x50(sp)",
    "    sd    x5,  0x58(sp)",
    "    sd    x6,  0x60(sp)",
    "    sd    x7,  0x68(sp)",
    "    sd    x8,  0x70(sp)",
    "    sd    x9,  0x78(sp)",
    "    sd    x10, 0x80(sp)",
    "    sd    x11, 0x88(sp)",
    "    sd    x12, 0x90(sp)",
    "    sd    x13, 0x98(sp)",
    "    sd    x14, 0xa0(sp)",
    "    sd    x15, 0xa8(sp)",
    "    sd    x16, 0xb0(sp)",
    "    sd    x17, 0xb8(sp)",
    "    sd    x18, 0xc0(sp)",
    "    sd    x19, 0xc8(sp)",
    "    sd    x20, 0xd0(sp)",
    "    sd    x21, 0xd8(sp)",
    "    sd    x22, 0xe0(sp)",
    "    sd    x23, 0xe8(sp)",
    "    sd    x24, 0xf0(sp)",
    "    sd    x25, 0xf8(sp)",
    "    sd    x26, 0x100(sp)",
    "    sd    x27, 0x108(sp)",
    "    sd    x28, 0x110(sp)",
    "    sd    x29, 0x118(sp)",
    "    sd    x30, 0x120(sp)",
    "    sd    x31, 0x128(sp)",
    "    csrr  t0, sscratch",
    "    sd    t0,  0x40(sp)",             // gpr[2] = 被中断内核 sp
    "    csrr  t0, sstatus",
    "    sd    t0,  0x130(sp)",
    "    csrr  t0, sepc",
    "    sd    t0,  0x138(sp)",
    // 经帧内元数据切到 per-hart trap 栈并进入 Rust handler
    "    ld    t2,  0x10(sp)",             // trap_handler
    "    ld    a0,  0x20(sp)",             // self_pa
    "    ld    sp,  0x08(sp)",             // kernel_sp = per-hart trap 栈顶
    "    jalr  t2",
    "    j     __restore",

    // ── 上下文恢复：a0 = 目标帧 self_pa（物理地址）────────────────────
    // 用户帧 / 内核帧统一走此路径：切表前（物理访问有效）读帧内 self_va，切表
    // 后经该 VA 访问同一物理页收尾。
    ".globl __restore",
    "__restore:",
    "    mv    sp, a0",
    "    ld    t0, 0x130(sp)",
    "    andi  t0, t0, -3",            // 清 sstatus.SIE：恢复后到 sret 之间不得被中断打断
    "    csrw  sstatus, t0",
    "    ld    t0, 0x138(sp)",
    "    csrw  sepc, t0",
    // sscratch 约定复原：SPP = 0（回用户）→ TRAP_CONTEXT VA；SPP = 1（回内核）→ 0
    "    csrr  t0, sstatus",
    "    andi  t0, t0, (1 << 8)",
    "    bnez  t0, 1f",
    "    ld    t0,  0x140(sp)",        // self_va（物理访问，切表前）
    "    csrw  sscratch, t0",
    "    j     2f",
    "1:",
    "    csrw  sscratch, zero",
    "2:",
    // 恢复 GPR（x1、x3、x4、x7..x31；x2=sp、x5=t0、x6=t1 最后经 TRAP_CONTEXT VA 收尾）
    "    ld    x1,  0x38(sp)",
    "    ld    x3,  0x48(sp)",
    "    ld    x4,  0x50(sp)",
    "    ld    x7,  0x68(sp)",
    "    ld    x8,  0x70(sp)",
    "    ld    x9,  0x78(sp)",
    "    ld    x10, 0x80(sp)",
    "    ld    x11, 0x88(sp)",
    "    ld    x12, 0x90(sp)",
    "    ld    x13, 0x98(sp)",
    "    ld    x14, 0xa0(sp)",
    "    ld    x15, 0xa8(sp)",
    "    ld    x16, 0xb0(sp)",
    "    ld    x17, 0xb8(sp)",
    "    ld    x18, 0xc0(sp)",
    "    ld    x19, 0xc8(sp)",
    "    ld    x20, 0xd0(sp)",
    "    ld    x21, 0xd8(sp)",
    "    ld    x22, 0xe0(sp)",
    "    ld    x23, 0xe8(sp)",
    "    ld    x24, 0xf0(sp)",
    "    ld    x25, 0xf8(sp)",
    "    ld    x26, 0x100(sp)",
    "    ld    x27, 0x108(sp)",
    "    ld    x28, 0x110(sp)",
    "    ld    x29, 0x118(sp)",
    "    ld    x30, 0x120(sp)",
    "    ld    x31, 0x128(sp)",
    "    ld    t0,  0x28(sp)",             // user_satp
    "    ld    t1,  0x140(sp)",            // self_va（切表前取，物理访问）
    "    csrw  satp, t0",
    "    sfence.vma",
    "    mv    x5,  t1",                   // x5 = self_va（目标空间帧 VA）
    "    ld    x6,  0x60(x5)",             // 原 t1
    "    ld    x2,  0x40(x5)",             // 目标 sp（用户栈 / 内核栈）
    "    ld    x5,  0x58(x5)",             // 原 t0（基址先读后写，合法）
    "    sret",

    ".globl __trampoline_end",
    "__trampoline_end:",
    tc = const TRAP_CONTEXT_LUI,
);

unsafe extern "C" {
    /// trampoline 页起始（链接符号：内核镜像恒等加载，链接地址即物理地址）。
    pub static __trampoline_start: u8;
    /// 陷阱入口（stvec 目标，位于本页内偏移）。
    pub static __alltraps: u8;
    /// trampoline 页结束（防呆：与 __trampoline_start 同页）。
    pub static __trampoline_end: u8;
    /// 上下文恢复出口（仅供 restore() 计算 TRAMPOLINE VA 偏移，勿直接链接地址调用）。
    pub static __restore: u8;
    /// per-hart trap 栈底 / 栈顶（link.ld 保留 16 KiB，恒等映射地址即物理地址）。
    pub static _trap_stack_bottom: u8;
    pub static _trap_stack_top: u8;
}

/// 恢复目标帧上下文并进入其中（永不返回）。frame_pa = 目标帧 self_pa。
///
/// 必须在 TRAMPOLINE VA 执行 `__restore`：切换用户页表后，链接地址（内核镜像）
/// 不再映射，只有 TRAMPOLINE VA 在目标空间恒映射（G 位）。
pub fn restore(frame_pa: usize) -> ! {
    let link = core::ptr::addr_of!(__restore) as usize;
    let va = TRAMPOLINE.as_usize() + (link - core::ptr::addr_of!(__trampoline_start) as usize);
    unsafe {
        core::arch::asm!(
            "jalr ra, {addr}",
            addr = in(reg) va,
            in("a0") frame_pa,
            options(noreturn),
        );
    }
}

/// trampoline 页的物理地址（链接符号地址：内核镜像 0x8020_0000 恒等加载）。
pub fn trampoline_pa() -> PhysAddr {
    PhysAddr::from_raw(core::ptr::addr_of!(__trampoline_start) as usize)
}

/// `__alltraps` 在 TRAMPOLINE 固定 VA 处的地址（stvec 写入值）。
pub fn alltraps_va() -> usize {
    let start = core::ptr::addr_of!(__trampoline_start) as usize;
    let alltraps = core::ptr::addr_of!(__alltraps) as usize;
    TRAMPOLINE.as_usize() + (alltraps - start)
}

/// per-hart trap 栈顶（阶段 A：hart 0 独占；多核按 hart 切分）。
pub fn trap_stack_top() -> usize {
    core::ptr::addr_of!(_trap_stack_top) as usize
}

/// per-hart trap 栈底（canary 写入处）。
pub fn trap_stack_bottom() -> usize {
    core::ptr::addr_of!(_trap_stack_bottom) as usize
}
