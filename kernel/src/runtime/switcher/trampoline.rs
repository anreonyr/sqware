// 陷阱 trampoline — 所有地址空间共同映射、共同取指的 trap 入口页（页身份）
//
// 一页（4 KiB）内含 `__alltraps`（保存帧 + 切 satp）与 `__restore`（切回 + 恢复 + sret），
// 内核空间与所有用户空间以 TRAMPOLINE VA 映射同一物理页（G 位），`stvec`
// 指向 `__alltraps`。
//
// 本页代码执行于 TRAMPOLINE 固定 VA（0xFFFF_FFFF_FFFF_F000）——任何 PC 相对寻址
// （la/call 等）的目标必须在本页内；跨页符号（如 Rust 的
// trap_handler）只能经帧内元数据（kernel_sp / trap_handler 字段）或绝对常量（LUI）
// 寻址。本页代码无 PC 相对跨页引用，故在链接地址（0x8020_0000+）与 TRAMPOLINE VA
// 两处取指均正确。hart 帧基址不再需要汇编常量：`__restore` 回内核复原 sscratch
// 时经 `tp` 指向的 `PerHart.frame` 字段单条 load 取本 hart 帧 VA（见 machine.rs，
// 布局偏移由编译期断言锁死）。
//
// 页的固有义务：汇编段必须落在一页内（TRAMPOLINE 映射只覆盖一页）——
// `check_fits_page` 在 boot 装配时校验（链接期才知尺寸，故为运行期断言）。
// 帧布局与偏移、sscratch 约定、per-hart trap 栈几何见 `context.rs` / `trap.rs`。

use core::arch::global_asm;

use crate::layout::TRAMPOLINE;
use crate::memory::PAGE_SIZE;

global_asm!(
    ".section .trampoline, \"ax\"",
    ".align 12",
    ".globl __trampoline_start",
    "__trampoline_start:",
    // ── 陷阱入口（stvec Direct 目标）──────────────────────────────────
    ".globl __alltraps",
    "__alltraps:",
    "    csrr  t0, sstatus",
    "    andi  t0, t0, (1 << 8)", // SPP：0 = 来自用户态，1 = 来自内核态
    "    bnez  t0, __strap",
    // ── 用户态陷阱（__utrap）：现场存当前线程帧（sscratch 交换）────────────
    "__utrap:",
    "    csrrw sp, sscratch, sp", // sp = 本线程帧 VA；sscratch = 用户 sp
    "    sd    x1,  0x38(sp)",    // gpr[1] = ra
    "    csrr  t0, sscratch",
    "    sd    t0,  0x40(sp)", // gpr[2] = 用户 sp
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
    "    sd    t0,  0x130(sp)", // sstatus
    "    csrr  t0, sepc",
    "    sd    t0,  0x138(sp)", // sepc
    // 读内核切换元数据（趁 sp 仍是用户帧、用户 satp 有效）
    "    ld    t0,  0x00(sp)", // kernel_satp
    "    ld    t1,  0x08(sp)", // kernel_sp
    "    ld    t2,  0x10(sp)", // trap_handler
    "    ld    a0,  0x20(sp)", // self_pa
    // 切内核页表（PC 仍在 trampoline，两空间同 VA，安全）
    "    csrw  satp, t0",
    "    sfence.vma",
    // 切内核栈；旧 sp（线程帧 VA）切表后指向本空间帧窗口，不再解引用。
    // （tp 由 C 侧 trap_handler 入口按 sp 反解重建——见 establish_tp；汇编
    //   不能 PC 相对引用跨页符号，TRAMPOLINE VA 下 la 会算出错误地址）
    "    mv    sp, t1",
    "    jalr  t2", // handler(frame_pa) -> frame_pa（续跑时恒为原帧）
    "    j     __restore",
    // ── 内核态陷阱（__strap）：现场存**本 hart**帧（PerHart.frame 定位——tp
    //    指向本 hart 上下文块，帧 VA 取块内字段，见 machine::PerHart；栈切本
    //    hart trap 栈。tp 约定：内核态恒为本 hart PerHart 指针——入口/
    //    establish_tp 维持。sscratch 内核态约定 = 本 hart 帧 VA，但**trap 入口
    //    不可靠**：用户 trap（__utrap）入口把 sscratch 换成用户 sp，处理中若有
    //    内核缺页再次进入本路径，sscratch 已被污染——故帧址仍由 tp 重建，
    //    不读 sscratch）。 ──
    "__strap:",
    "    csrrw sp, sscratch, sp", // sp = 0（内核态约定）；sscratch = 被中断内核 sp
    "    ld    sp, 0x08(tp)",     // sp = PerHart.frame（本 hart 帧 VA）
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
    "    sd    t0,  0x40(sp)", // gpr[2] = 被中断内核 sp
    "    csrr  t0, sstatus",
    "    sd    t0,  0x130(sp)",
    "    csrr  t0, sepc",
    "    sd    t0,  0x138(sp)",
    // 经帧内元数据切到 per-hart trap 栈并进入 Rust handler
    "    ld    t2,  0x10(sp)", // trap_handler
    "    ld    a0,  0x20(sp)", // self_pa
    "    ld    sp,  0x08(sp)", // kernel_sp = per-hart trap 栈顶
    "    jalr  t2",
    "    j     __restore",
    // ── 上下文恢复：a0 = 目标帧 self_pa（物理地址）────────────────────
    // 用户帧 / hart 帧统一走此路径：切表前（物理访问有效）读帧内 self_va，切表
    // 后经该 VA 访问同一物理页收尾。
    ".globl __restore",
    "__restore:",
    "    mv    sp, a0",
    "    ld    t0, 0x130(sp)",
    "    andi  t0, t0, -3", // 清 sstatus.SIE：恢复后到 sret 之间不得被中断打断
    "    csrw  sstatus, t0",
    "    ld    t0, 0x138(sp)",
    "    csrw  sepc, t0",
    // sscratch 约定复原：SPP = 0（回用户）→ 线程帧 self_va；SPP = 1（回内核）→
    // 本 hart 帧 VA（PerHart.frame；tp 此刻未被帧覆盖，仍是执行核 PerHart 指针
    // ——x4 随后才从帧恢复；跨核迁移时即取**执行核**的帧）
    "    csrr  t0, sstatus",
    "    andi  t0, t0, (1 << 8)",
    "    bnez  t0, 1f",
    "    ld    t0,  0x140(sp)", // self_va（物理访问，切表前）
    "    csrw  sscratch, t0",
    "    j     2f",
    "1:",
    "    ld    t0, 0x08(tp)", // t0 = PerHart.frame（本 hart 帧 VA）
    "    csrw  sscratch, t0",
    "2:",
    // 恢复 GPR（x1、x3、x4、x7..x31；x2=sp、x5=t0、x6=t1 最后经 self_va 收尾）
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
    "    ld    t0,  0x28(sp)",  // user_satp
    "    ld    t1,  0x140(sp)", // self_va（切表前取，物理访问）
    "    csrw  satp, t0",
    "    sfence.vma",
    "    mv    x5,  t1",       // x5 = self_va（目标空间帧 VA）
    "    ld    x6,  0x60(x5)", // 原 t1
    "    ld    x2,  0x40(x5)", // 目标 sp（用户栈 / 内核栈）
    "    ld    x5,  0x58(x5)", // 原 t0（基址先读后写，合法）
    "    sret",
    ".globl __trampoline_end",
    "__trampoline_end:",
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

/// `__alltraps` 在 TRAMPOLINE 固定 VA 处的地址（stvec 写入值）。
pub fn alltraps_va() -> usize {
    let start = core::ptr::addr_of!(__trampoline_start) as usize;
    let alltraps = core::ptr::addr_of!(__alltraps) as usize;
    TRAMPOLINE.as_usize() + (alltraps - start)
}

/// 页义务防呆：汇编段必须落在一页内（TRAMPOLINE 映射只覆盖一页）。
/// boot 装配时（trap::init）调用恰好一次。
pub fn check_fits_page() {
    let tsize = (core::ptr::addr_of!(__trampoline_end) as usize)
        - (core::ptr::addr_of!(__trampoline_start) as usize);
    assert!(
        tsize <= PAGE_SIZE,
        "trampoline exceeds one page: {tsize:#x}"
    );
}
