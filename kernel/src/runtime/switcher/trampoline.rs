// 陷阱 trampoline — 所有地址空间共同映射、共同取指的 trap 入口页
//
// 一页（4 KiB）内含 `__alltraps`（保存帧 + 切 satp）与 `__restore`（切回 + 恢复 + sret），
// 内核空间与所有用户空间以 TRAMPOLINE VA 映射同一物理页（G 位），`stvec`
// 指向 `__alltraps`。
//
// 本页代码执行于 TRAMPOLINE 固定 VA（0xFFFF_FFFF_FFFF_F000）——任何 PC 相对寻址
// （la/call 等）的目标必须在本页内；跨页符号（如 Rust 的
// trap_handler）只能经帧内元数据（kernel_sp / trap_handler 字段）或绝对常量（LUI）
// 寻址。本页代码无 PC 相对跨页引用，故在链接地址（0x8020_0000+）与 TRAMPOLINE VA
// 两处取指均正确。per-hart 内核帧基址（__restore 回内核时复原 sscratch 用）的 LUI
// 立即数由 Rust 常量 KERNEL_FRAMES_LUI 注入（单一来源，改 KERNEL_FRAME_BASE 即可）。
//
// sscratch 约定：用户态 = 当前线程帧 VA（帧内 self_va 字段，帧窗口分配，无固定帧 VA）；
// 内核态 = 本 hart 内核 trap 帧 VA（KERNEL_FRAME_BASE + hart·PAGE）——boot 与
// __restore 的 SPP=1 分支维护。`__restore` 按恢复的 sstatus.SPP 复原该约定
// （SPP=0 从帧内 self_va 字段读取——每线程帧位置可任意，`__alltraps` 零改动）。
//
// trap 栈与推 hart：per-hart trap 栈段位于固定 VA 窗口
// （TRAP_STACK_BASE + hart·64 KiB，见 `space::TRAP_STACK_*`）——入口（tp 重建前）
// 即可按当前 sp 纯算术反解 hart；不依赖任何动态元数据表。
//
// 帧布局与偏移见 `context.rs`（编译期偏移断言锁定，改布局必须先改两处）。

use alloc::vec::Vec;
use core::arch::global_asm;

use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{
    KERNEL_FRAME_BASE, TRAMPOLINE, TRAP_STACK_BASE, TRAP_STACK_GUARD, TRAP_STACK_SEGMENT,
    TRAP_STACK_SHIFT,
};

/// KERNEL_FRAME_BASE 的 LUI 立即数（bits[31:12]）——__strap 按 TP 索引 per-hart
/// 内核帧：sp = KERNEL_FRAME_BASE + tp·PAGE_SIZE。汇编经 `const` 注入，单一来源。
///
/// LUI 把 20 位立即数符号扩展后左移 12 位，与 `VirtAddr::from_raw` 的符号扩展
/// 语义一致；改 `KERNEL_FRAME_BASE` 即可，勿手改汇编。
const KERNEL_FRAMES_LUI: usize = (KERNEL_FRAME_BASE.as_usize() >> 12) & 0xFFFFF;

// 编码断言：LUI 立即数符号扩展必须能还原 KERNEL_FRAME_BASE（VA 不再满足 LUI
// 编码时编译期报错）。
const _: () = {
    let shift = usize::BITS as usize - 20;
    let imm = ((KERNEL_FRAMES_LUI as isize) << shift) >> shift;
    assert!(((imm as usize) << 12) == KERNEL_FRAME_BASE.as_usize());
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

    // ── 用户态陷阱（__utrap）：现场存当前线程帧（sscratch 交换）────────────
    "__utrap:",
    "    csrrw sp, sscratch, sp",          // sp = 本线程帧 VA；sscratch = 用户 sp
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
    // 切内核栈；旧 sp（线程帧 VA）切表后指向本空间帧窗口，不再解引用。
    // （tp 由 C 侧 trap_handler 入口按 sp 反解重建——见 establish_tp；汇编
    //   不能 PC 相对引用跨页符号，TRAMPOLINE VA 下 la 会算出错误地址）
    "    mv    sp, t1",
    "    jalr  t2",                        // handler(frame_pa) -> frame_pa（续跑时恒为原帧）
    "    j     __restore",

    // ── 内核态陷阱（__strap）：现场存**本 hart**内核帧（TP 索引 per-hart 帧区，
    //    栈切本 hart trap 栈。tp 约定：内核态恒为 hartid——入口/establish_tp 维持。
    //    sscratch 内核态约定 = 本 hart 内核帧 VA，但**trap 入口不可靠**：用户 trap
    //    （__utrap）入口把 sscratch 换成用户 sp，处理中若有内核缺页再次进入本路径，
    //    sscratch 已被污染——故帧址仍由 tp 重建，不读 sscratch）。 ──
    "__strap:",
    "    csrrw sp, sscratch, sp",          // sp = 0（内核态约定）；sscratch = 被中断内核 sp
    "    slli  t0, tp, 12",               // t0 = hart · PAGE_SIZE（per-hart 帧索引）
    "    lui   sp, {kf}",                 // sp = KERNEL_FRAME_BASE（LUI 高 20 位）
    "    add   sp, sp, t0",               // sp = 本 hart 内核帧 VA（内核空间映射）
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
    // sscratch 约定复原：SPP = 0（回用户）→ 线程帧 self_va；SPP = 1（回内核）→
    // 本 hart 内核帧 VA（KERNEL_FRAME_BASE + hart·PAGE；tp 此刻未被帧覆盖，
    // 仍是执行核 hartid——x4 随后才从帧恢复；跨核迁移时即取**执行核**的帧）
    "    csrr  t0, sstatus",
    "    andi  t0, t0, (1 << 8)",
    "    bnez  t0, 1f",
    "    ld    t0,  0x140(sp)",        // self_va（物理访问，切表前）
    "    csrw  sscratch, t0",
    "    j     2f",
    "1:",
    "    slli  t0, tp, 12",           // t0 = hart · PAGE_SIZE
    "    lui   t1, {kf}",             // t1 = KERNEL_FRAME_BASE（LUI 高 20 位）
    "    add   t0, t0, t1",           // t0 = 本 hart 内核帧 VA
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
    kf = const KERNEL_FRAMES_LUI,
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

// ── per-hart trap 栈：固定 VA 窗口 + 纯算术反解（无元数据表）──
//
// 布局（space::TRAP_STACK_*）：TRAP_STACK_BASE 起，hart h 段 =
//   TRAP_STACK_BASE + h·SEGMENT：首页 guard（内核空间未映射，越界即页故障）、
//   其下 60 KiB 栈体（boot 时映射物理页）。
// 反解：段大小 = 2^SHIFT ⇒ hart = (sp − BASE) >> SHIFT（O(1)、零表、零堆依赖——
//   堆破坏不再能经元数据表污染 hart 判定）。崩溃路径（scene 钳制、guard 识别）
//   与正常路径同源，均不依赖任何运行时表。

/// 物理块基址（boot 时 frame 连续分配 N×64 KiB，段物理首址 = base + h·SEGMENT）。
/// 仅副核 HSM 启动栈（bare 模式，sp 必须是物理地址）使用；trap 侧一律走固定 VA。
static TRAP_STACK_PHYS: OnceLock<usize> = OnceLock::new();

/// hart h 的 trap 栈段几何（纯算术）：守卫页不映射，栈体 = [base+GUARD, base+SEGMENT)。
fn trap_stack_seg(hart: usize) -> (usize, usize) {
    let base = TRAP_STACK_BASE.as_usize() + hart * TRAP_STACK_SEGMENT;
    (base + TRAP_STACK_GUARD, base + TRAP_STACK_SEGMENT) // (bottom, top)
}

/// sp 是否落在某 hart 的 trap 栈体内（guard 之上、top 之下→含）——反解 hart。
///
/// 崩溃路径的瘦身版 `establish_tp`：推 hart 不读表、不 panic；越出窗口/guard/
/// 未启用核一律 None（引导期与非法现场合法返回）。正常路径恒命中：trap handler
/// 恒在 per-hart trap 栈上执行。
pub(crate) fn hart_of_trap_stack(sp: usize) -> Option<usize> {
    let off = sp.checked_sub(TRAP_STACK_BASE.as_usize())?;
    let h = off >> TRAP_STACK_SHIFT;
    if h >= crate::machine::hart_count() {
        return None;
    }
    let in_seg = off & (TRAP_STACK_SEGMENT - 1);
    (in_seg > TRAP_STACK_GUARD && in_seg <= TRAP_STACK_SEGMENT).then_some(h)
}

/// 由当前 sp（trap 栈体内）反解 hart 并写入 tp——用户态可自由改写 tp，而内核
/// 调度/锁/canary 全部依赖 hart_id()（读 tp）；trap 入口必须先重建。
///
/// 汇编不能做这件事：trampoline 执行于 TRAMPOLINE VA，PC 相对引用跨页符号会
/// 算出错误地址（本页代码的寻址约束，见模块头注释）。C 代码执行于链接地址，
/// 固定布局纯算术反解无此问题。异常 sp（窗口外/guard/早期 boot）回退 0，
/// 与既有 `unwrap_or(0)` 语义一致。
pub fn establish_tp() {
    let sp: usize;
    // SAFETY: 读当前栈指针，纯读无副作用。
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    let hart = hart_of_trap_stack(sp).unwrap_or(0);
    // SAFETY: 写线程指针寄存器（仅 trap 入口调用一次，重建内核 hartid）。
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) hart, options(nomem, nostack, preserves_flags));
    }
}

/// hart 的 trap 栈顶（固定 VA；__alltraps/__strap 经帧内 kernel_sp 上栈的目标）。
pub fn trap_stack_top(hart: usize) -> usize {
    trap_stack_seg(hart).1
}

/// hart 的 trap 栈体底（固定 VA，canary 处）。
pub fn trap_stack_bottom(hart: usize) -> usize {
    trap_stack_seg(hart).0
}

/// 地址是否落在某 hart 的 trap 栈 guard 页内（返回该 hart 号）——内核故障
/// 路径据此识别「trap 栈溢出」并给出精确诊断。纯算术，不读表。
pub fn trap_stack_guard_hart(addr: usize) -> Option<usize> {
    let off = addr.checked_sub(TRAP_STACK_BASE.as_usize())?;
    if off & (TRAP_STACK_SEGMENT - 1) < TRAP_STACK_GUARD {
        let h = off >> TRAP_STACK_SHIFT;
        (h < crate::machine::hart_count()).then_some(h)
    } else {
        None
    }
}

/// 副核 HSM 启动栈顶（**物理地址**，bare 模式可用）。boot_harts 专用；点火后
/// 恒等/固定 VA 两路均可达同一物理页，trap 侧别再经此取址。
pub fn trap_stack_phys_top(hart: usize) -> usize {
    TRAP_STACK_PHYS.get().expect("trap stacks not initialized") + (hart + 1) * TRAP_STACK_SEGMENT
}

/// 初始化 per-hart trap 栈（boot 时调用**恰好一次**，hart 0）。
///
/// 1. 按实际核数（DTB）frame **连续**分配 N x 64 KiB（frame 按 order 支持连续块，
///    向上取整到 2 的幂）；物理基址存入 TRAP_STACK_PHYS（副核启动栈用）；
/// 2. 内核空间把各段**栈体**映射到固定 VA 窗口（guard 页不映射——越界即页故障）；
/// 3. 恒等视图 guard 页清 PTE（副核 boot 栈溢出的既有护栏，行为不变）；
/// 4. 各段栈底（固定 VA）写 canary。
///
/// 块永不释放（静态分配，段数 = 实际核数）；无元数据表。
pub fn init_trap_stacks() {
    let segments = crate::machine::hart_count();
    assert!(segments > 0, "no harts");
    assert_eq!(
        TRAP_STACK_SEGMENT,
        1 << TRAP_STACK_SHIFT,
        "trap stack segment must be 2^SHIFT"
    );
    let total = segments * TRAP_STACK_SEGMENT;
    let layout = core::alloc::Layout::from_size_align(total, PAGE_SIZE).expect("trap stack layout");
    // 块连续（frame 按 order 取整到 2 的幂）；boot 期帧池充足。
    let block = allocator()
        .allocate(layout)
        .expect("trap stack block allocation");
    let base = block.cast::<u8>().as_ptr() as usize;
    assert!(
        TRAP_STACK_PHYS.set(base).is_ok(),
        "trap stack phys double init"
    );

    let space = &crate::work::unit::team::kernel()
        .expect("kernel team not initialized")
        .space;
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    for h in 0..segments {
        let (body_va, top_va) = trap_stack_seg(h);
        let phys = base + h * TRAP_STACK_SEGMENT;
        // 段体映射（60 KiB）：固定 VA → 块内物理页；guard 页不映射（越界即页故障）
        space
            .map(
                VirtAddr::from_raw(body_va),
                PhysAddr::from_raw(phys + TRAP_STACK_GUARD),
                TRAP_STACK_SEGMENT - TRAP_STACK_GUARD,
                flags,
                crate::work::unit::space::MapKind::Reserved,
                Vec::new(),
            )
            .expect("map trap stack body");
        // 恒等视图 guard 页清 PTE 保留 boot 栈溢出护栏（固定 VA guard 管 trap 栈）
        space.unmap(VirtAddr::from_raw(phys), TRAP_STACK_GUARD);
        // canary 写于固定 VA 栈体底（guard 之上）
        unsafe {
            (body_va as *mut usize).write(crate::runtime::switcher::trap::TRAP_STACK_CANARY);
        }
        debug_assert!(top_va == trap_stack_top(h));
    }
}
