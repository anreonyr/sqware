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
// 两处取指均正确。per-hart 内核帧基址（__strap 按 TP 索引）的 LUI 立即数由
// Rust 常量 KERNEL_FRAMES_LUI 注入（单一来源，改 space::KERNEL_FRAME_BASE 即可）。
//
// sscratch 约定：用户态 = 当前线程帧 VA（帧内 self_va 字段，帧窗口分配，不再
// 固定 TRAP_CONTEXT）；内核态 = 0。`__restore` 按恢复的 sstatus.SPP 复原该约定
// （SPP=0 从帧内 self_va 字段读取——每线程帧位置可任意，`__alltraps` 零改动）。
//
// 帧布局与偏移见 runtime/context.rs（编译期偏移断言锁定，改布局必须先改两处）。

use alloc::boxed::Box;
use core::arch::global_asm;
use core::cell::UnsafeCell;

use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::space::{KERNEL_FRAME_BASE, TRAMPOLINE, kernel_space};

/// KERNEL_FRAME_BASE 的 LUI 立即数（bits[31:12]）——__strap 按 TP 索引 per-hart
/// 内核帧：sp = KERNEL_FRAME_BASE + tp·PAGE_SIZE。汇编经 `const` 注入，单一来源。
///
/// LUI 把 20 位立即数符号扩展后左移 12 位，与 `VirtAddr::from_raw` 的符号扩展
/// 语义一致；改 `space::KERNEL_FRAME_BASE` 即可，勿手改汇编。
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
    // 切内核栈；旧 sp（TRAP_CONTEXT VA）切表后指向内核帧，不再解引用。
    // （tp 由 C 侧 trap_handler 入口按 sp 反解重建——见 establish_tp；汇编
    //   不能 PC 相对引用跨页符号，TRAMPOLINE VA 下 la 会算出错误地址）
    "    mv    sp, t1",
    "    jalr  t2",                        // handler(frame_pa) -> frame_pa（阶段 A 恒原帧）
    "    j     __restore",

    // ── 内核态陷阱（__strap）：现场存**本 hart**内核帧（TP 索引 per-hart 帧区，
    //    栈切本 hart trap 栈。tp 约定：内核态恒为 hartid——入口/establish_tp 维持；
    //    用户态可改写 tp，但内核陷阱只在 S 态发生）。 ──
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

/// per-hart trap 栈段元数据（boot 时由 hart 0 的 [`init_trap_stacks`] 一次性
/// 填充，此后只读；UnsafeCell：静态不可变处经原始指针写入一次）。
#[derive(Clone, Copy, Debug)]
pub struct TrapStackMeta {
    /// 栈顶（向下增长，sp 初始值）。
    pub top: usize,
    /// 栈体底（canary 所在，guard 之上）。
    pub bottom: usize,
    /// guard 页起始（内核空间未映射，越过即页故障）。
    pub guard: usize,
}

/// 写一次、此后只读的 Sync 单元（trap 栈表 boot 时由 hart 0 填充）。
struct SyncCell<T>(UnsafeCell<T>);

// SAFETY: 调用方保证 boot 时单写者填充一次、此后只读。
unsafe impl<T: Copy> Sync for SyncCell<T> {}

impl<T: Copy> SyncCell<T> {
    const fn new(v: T) -> Self {
        Self(UnsafeCell::new(v))
    }
    fn get(&self) -> T {
        // SAFETY: 只读访问（boot 后不再写）。
        unsafe { *self.0.get() }
    }
    fn set(&self, v: T) {
        // SAFETY: 仅 boot 时由 hart 0 调用一次。
        unsafe {
            *self.0.get() = v;
        }
    }
}

/// per-hart trap 栈元数据表（模块内专用；经 trap_stack_top/bottom 读取）。
///
/// **按实际核数动态分配**（boot 时 init_trap_stacks 分配并填充；此后只读）。
static TRAP_STACKS: OnceLock<&'static [SyncCell<TrapStackMeta>]> = OnceLock::new();

/// 读取某 hart 的 trap 栈元数据。
fn trap_stack_meta(hart: usize) -> TrapStackMeta {
    TRAP_STACKS.get().expect("trap stacks not initialized")[hart].get()
}

/// trap 栈元数据表长度（= 实际核数）。
fn trap_stack_count() -> usize {
    TRAP_STACKS
        .get()
        .expect("trap stacks not initialized")
        .len()
}

/// 由当前 sp（trap 栈体内）反解 hart 并写入 tp——用户态可自由改写 tp，而内核
/// 调度/锁/canary 全部依赖 hart_id()（读 tp）；trap 入口必须先重建。
///
/// 汇编不能做这件事：trampoline 执行于 TRAMPOLINE VA，PC 相对引用跨页符号会
/// 算出错误地址（本页代码的寻址约束，见模块头注释）。C 代码执行于链接地址，
/// 反解 TRAP_STACKS 表无此问题。
pub fn establish_tp() {
    let sp: usize;
    // SAFETY: 读当前栈指针，纯读无副作用。
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    let hart = (0..trap_stack_count())
        .find(|&h| {
            let m = trap_stack_meta(h);
            m.top != 0 && sp <= m.top && sp > m.bottom
        })
        .unwrap_or(0);
    // SAFETY: 写线程指针寄存器（仅 trap 入口调用一次，重建内核 hartid）。
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) hart, options(nomem, nostack, preserves_flags));
    }
}

/// hart 的 trap 栈顶（__alltraps/__strap 经帧内 kernel_sp 上栈的目标）。
pub fn trap_stack_top(hart: usize) -> usize {
    trap_stack_meta(hart).top
}

/// hart 的 trap 栈体底（canary 处）。
pub fn trap_stack_bottom(hart: usize) -> usize {
    trap_stack_meta(hart).bottom
}

/// 地址是否落在某 hart 的 trap 栈 guard 页内（返回该 hart 号）——内核故障
/// 路径据此识别「trap 栈溢出」并给出精确诊断。
pub fn trap_stack_guard_hart(addr: usize) -> Option<usize> {
    for h in 0..trap_stack_count() {
        let guard = trap_stack_meta(h).guard;
        if guard != 0 && addr >= guard && addr < guard + PAGE_SIZE {
            return Some(h);
        }
    }
    None
}

/// per-hart trap 栈段常量：每段 32 KiB = 低 4 KiB guard（未映射）+ 28 KiB 栈体；
/// 连续块按实际核数分配（N x 32 KiB，frame 向上取整到 2 的幂），guard 页兼作
/// 段边界。
pub const TRAP_STACK_SEGMENT: usize = 32 * 1024;
pub const TRAP_STACK_GUARD: usize = PAGE_SIZE;

/// 初始化 per-hart trap 栈（boot 时由 trap::init 调用**恰好一次**，hart 0）。
///
/// 1. 按实际核数（DTB）frame **连续**分配 N x 32 KiB（frame 按 order 支持连续块，
///    向上取整到 2 的幂）；
/// 2. 动态分配 TRAP_STACKS 元数据表（N 项）并置入 OnceLock；
/// 3. 内核空间**清 guard 页 PTE**（栈体随 DRAM 恒等映射保持可访问——unmap 对
///    部分覆盖的 DRAM 常数映射只清 PTE、保留簿记）；
/// 4. 各段栈底写 canary；填充表项。
///
/// 块与表永不释放（静态分配，段数 = 实际核数）。
pub fn init_trap_stacks() {
    let segments = crate::machine::hart_count();
    assert!(segments > 0, "no harts");
    let total = segments * TRAP_STACK_SEGMENT;
    let layout = core::alloc::Layout::from_size_align(total, PAGE_SIZE).expect("trap stack layout");
    // 块连续（frame 按 order 取整到 2 的幂）；boot 期帧池充足。
    let block = allocator()
        .allocate(layout)
        .expect("trap stack block allocation");
    let base = block.cast::<u8>().as_ptr() as usize;
    // establish_tp 反解 hartid 依赖段大小 = 2^15（kernel_sp - base 恒为 32 KiB
    // 整数倍——段连续切分；C 侧按 TRAP_STACKS 表范围匹配，无需 base 静态）
    assert_eq!(
        TRAP_STACK_SEGMENT,
        1 << 15,
        "trap stack segment must be 32 KiB"
    );

    // 元数据表（N 项，先置 OnceLock 再逐项填充）
    let table: Box<[SyncCell<TrapStackMeta>]> = (0..segments)
        .map(|_| {
            SyncCell::new(TrapStackMeta {
                top: 0,
                bottom: 0,
                guard: 0,
            })
        })
        .collect();
    assert!(
        TRAP_STACKS.set(Box::leak(table)).is_ok(),
        "trap stacks double init"
    );

    let ks = kernel_space();
    let space = ks.as_ref().expect("kernel space not initialized");
    for h in 0..segments {
        let seg = base + h * TRAP_STACK_SEGMENT;
        let guard = seg;
        let body = seg + TRAP_STACK_GUARD;
        // guard 页：内核空间清 PTE（未映射 → 越过即页故障）
        space.unmap(VirtAddr::from_raw(guard), TRAP_STACK_GUARD);
        // canary 写于栈体底（guard 之上）
        unsafe {
            (body as *mut usize).write(crate::runtime::trap::TRAP_STACK_CANARY);
        }
        trap_stack_cell(h).set(TrapStackMeta {
            top: seg + TRAP_STACK_SEGMENT,
            bottom: body,
            guard,
        });
    }
}

/// 表项引用（boot 填充用：SyncCell::set 取 &self）。
fn trap_stack_cell(hart: usize) -> &'static SyncCell<TrapStackMeta> {
    &TRAP_STACKS.get().expect("trap stacks not initialized")[hart]
}
