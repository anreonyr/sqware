// 地址空间内存布局 — 内核/用户虚拟空间几何的单一事实源。
//
// 改任何布局只改本文件（编译期断言锁死「对齐/相邻/不重叠」，运行期 validate
// 锁死随模式部分），并同步 link.ld / trampoline 汇编。
//
// 总览（**从高到低**排布；几何随模式，见 memory::manager::mode::lower/upper）：
//
// ┌─ 虚拟地址空间（顶锚 = TRAMPOLINE；Sv39 高半区顶部 336 MiB 窗口区）
// │
// ├─ [内核半区] lower .. TRAMPOLINE（段间相接不重叠）
// │  ├─ TRAMPOLINE 页 4 KiB    0xFFFF_FFFF_FFFF_F000 ← 最高页（任何模式）
// │  │   （其上 2 MiB−4KiB 间隙留空：TRAMPOLINE 独占最后 2 MiB 区顶部，
// │  │     下方窗口区自 2 MiB 边界起排——全部对齐，页表层级干净）
// │  ├─ hart 帧区 16 MiB   0xFFFF_FFFF_FEE0_0000 ← HART_FRAME_BASE
// │  │  └─ 4096 槽 × 4 KiB     HART_FRAME_SLOTS（每 hart 一帧）
// │  ├─ team 帧区 64 MiB    0xFFFF_FFFF_FAE0_0000 ← TEAM_FRAME_BASE
// │  │  └─ 16384 页            TEAM_FRAME_WINDOW_SIZE（线程 trap 帧）
// │  └─ trap 栈区 256 MiB   0xFFFF_FFFF_EAE0_0000 ← TRAP_STACK_BASE
// │     └─ 4096 槽 × 64 KiB    TRAP_STACK_SLOT_SIZE（guard 4 KiB + 栈体 60 KiB）
// │
// ├─（lower/upper：内核半区 ∥ 用户半区分界，处规范空洞）
// │
// └─ [用户半区] 0x0000_0000 .. upper（upper = 1 << split_bit）
//    ├─ 任务栈窗口 1 GiB       upper − STACK_WINDOW_SIZE ← 顶锚
//    │  └─ 任务栈 slot         TASK_STACK_GUARD(4 KiB) + TASK_STACK_SIZE(16 KiB)
//    │
//    ├─ 用户堆                 [image_end, upper − 1 GiB)
//    │
//    └─ 程序镜像 64 KiB        0x1_0000 ← IMAGE_BASE
//
// ┌─ 物理内存（恒等加载，从高到低）
// ├─ free DRAM              [root_stack_edge, dram_end)
// ├─ ROOT 栈 64 KiB         [_kernel_edge, +ROOT_STACK_SIZE)（_kernel_edge 顶锚向下，
// │                         栈底 ROOT_STACK_CANARY；boot 期主栈 / panic 救援栈）
// └─ 内核镜像               [_kernel_start, _kernel_edge)（trampoline 页双映射：
//                           链接地址 + TRAMPOLINE VA）

use crate::machine::MAX_HART_SLOTS;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
#[cfg(debug_assertions)]
use crate::memory::manager::mode;

// ── 栈尺寸 ─────────────────────────────────────────────────

/// 单任务栈 16 KiB。
pub(crate) const TASK_STACK_SIZE: usize = 16384;
/// 任务栈守护页大小（= 一页）。
pub(crate) const TASK_STACK_GUARD: usize = PAGE_SIZE;
/// ROOT 栈 64 KiB（`_kernel_edge` 顶锚向下；boot 期主栈，panic 时作救援栈）。
pub(crate) const ROOT_STACK_SIZE: usize = 0x1_0000;

// ── 内核半区 VA（自 TRAMPOLINE 顶锚向下排布）───────────────

/// trap 入口页固定 VA（任何模式最高页，规范）。
pub(crate) const TRAMPOLINE: VirtAddr = VirtAddr::wrap(0xFFFF_FFFF_FFFF_F000);

/// 窗口区顶锚 = TRAMPOLINE 页之下首个 2 MiB 边界（TRAMPOLINE 独占最后 2 MiB
/// 区顶部 4 KiB；窗口区自 0xFFFF_FFFF_FFE0_0000 起，**全部窗口 2 MiB 对齐**）。
///
/// 为何对齐：Sv39 页表层级中 2 MiB 是 level-1 大页粒度（1 GiB 是 level-2 粒度）——
/// 窗口基址对齐 2 MiB 让页表子树边界干净、可整窗用到 level-1 大页；且三窗口 +
/// TRAMPOLINE 全落顶部 1 GiB 区（0xFFFF_FFFF_C000_0000 之上 336 MiB），level-2
/// 单条覆盖全部窗口，中间表共享最大化。
pub(crate) const KERNEL_TOP: VirtAddr =
    VirtAddr::wrap(TRAMPOLINE.as_usize() - (2 * 1024 * 1024 - PAGE_SIZE));

/// trampoline 页物理地址（链接符号 __trampoline_start）。
pub(crate) fn trampoline_pa() -> PhysAddr {
    unsafe extern "C" {
        static __trampoline_start: u8;
    }
    PhysAddr::from_raw(core::ptr::addr_of!(__trampoline_start) as usize)
}

/// hart 帧槽数 = MAX_HART_SLOTS（4096 页 = 16 MiB 窗口）。
pub(crate) const HART_FRAME_SLOTS: usize = MAX_HART_SLOTS;
/// hart 帧区基址 = KERNEL_TOP − SLOTS·PAGE_SIZE（16 MiB，2 MiB 对齐）。
pub(crate) const HART_FRAME_BASE: VirtAddr =
    VirtAddr::wrap(KERNEL_TOP.as_usize() - HART_FRAME_SLOTS * PAGE_SIZE);
/// team 帧区大小 = 64 MiB（16384 帧；2 MiB 倍数，整窗可用 level-1 大页）。
pub(crate) const TEAM_FRAME_WINDOW_SIZE: usize = 64 * 1024 * 1024;
/// team 帧区基址 = HART_FRAME_BASE − TEAM_FRAME_WINDOW_SIZE（64 MiB，2 MiB 对齐）。
pub(crate) const TEAM_FRAME_BASE: VirtAddr =
    VirtAddr::wrap(HART_FRAME_BASE.as_usize() - TEAM_FRAME_WINDOW_SIZE);

/// trap 栈每 hart 段 64 KiB（1 guard 页 + 60 KiB 栈体）。
pub(crate) const TRAP_STACK_SLOT_SIZE: usize = 64 * 1024;
/// 段位移 = log2(64 KiB)。
pub(crate) const TRAP_STACK_SLOT_SHIFT: usize = 16;
/// trap 栈段 guard 页（= 一页）。
pub(crate) const TRAP_STACK_GUARD: usize = PAGE_SIZE;
/// trap 栈窗口基址 = TEAM_FRAME_BASE − MAX_HART_SLOTS·64 KiB（256 MiB，2 MiB 对齐）。
pub(crate) const TRAP_STACK_BASE: VirtAddr =
    VirtAddr::wrap(TEAM_FRAME_BASE.as_usize() - (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT));

// ── 用户半区 ────────────────────────────────────────────────

/// 用户栈窗口 1 GiB（顶锚于 mode::upper）。
pub const STACK_WINDOW_SIZE: usize = 0x4000_0000;
/// 用户程序加载基址 64 KiB。
pub const IMAGE_BASE: VirtAddr = VirtAddr::wrap(0x1_0000);

// ── 布局即不变量：编译期断言锁死「对齐 / 相邻 / 不重叠」（模式无关部分）──
//
// 改布局必须先改这里（编译器兜底），并同步 link.ld / trampoline 汇编。
// 模式相关部分（lower/upper/堆栈几何）由 [`validate`] 运行期校验。
const _: () = {
    // 注意：VirtAddr 的 Add/Sub/PartialEq 非 const fn，此处一律用 as_usize() 裸算术。
    assert!(TRAMPOLINE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(HART_FRAME_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    // 窗口区顶锚：TRAMPOLINE 之下 2 MiB 边界（TRAMPOLINE 独占最后 2 MiB 区顶部
    // 4 KiB，间隙 2 MiB−4 KiB 留空——VA 免费，换取全部窗口 2 MiB 对齐）。
    assert!(KERNEL_TOP.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TRAMPOLINE.as_usize() - KERNEL_TOP.as_usize() == 2 * 1024 * 1024 - PAGE_SIZE);
    // hart 帧区：KERNEL_TOP 起 SLOTS 页向下排布，恰止于 HART_FRAME_BASE
    assert!(HART_FRAME_BASE.as_usize() + HART_FRAME_SLOTS * PAGE_SIZE == KERNEL_TOP.as_usize());
    // 帧窗口：2 MiB 对齐、恰止于 HART_FRAME_BASE（hart 帧区在其上方，互不重叠）
    assert!(HART_FRAME_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_WINDOW_SIZE.is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE == HART_FRAME_BASE.as_usize());
    assert!(TASK_STACK_SIZE.is_multiple_of(PAGE_SIZE));
    // trap 栈窗口：base 2 MiB 对齐；窗口不越过 TEAM_FRAME_BASE（下沿恰与帧窗口衔接）
    assert!(TRAP_STACK_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(
        TRAP_STACK_BASE.as_usize() + (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT)
            == TEAM_FRAME_BASE.as_usize()
    );
    // 段几何自洽：64 KiB = 2^16；guard = 一页；段 = guard + 栈体
    assert!(TRAP_STACK_SLOT_SIZE == 1usize << TRAP_STACK_SLOT_SHIFT);
    assert!(TRAP_STACK_GUARD == PAGE_SIZE);
};

/// 运行期布局校验（boot 后经 unit::init 调用）：模式几何与布局不变量。
///
/// debug 构建违例 fail-fast；release 空体。const 断言块管模式无关部分，
/// 本校验管随模式部分（lower / upper / 用户/内核几何）。
#[cfg(debug_assertions)]
pub(crate) fn validate() {
    let geo = mode::geometry(mode::mode());
    let split = geo.split_bit() as usize;
    let top = 1usize << split;
    let lower = mode::lower();
    let upper = mode::upper();
    // 几何自洽：va_bits = 12 + 9·levels
    assert!(
        (3..=5).contains(&geo.levels) && geo.va_bits as usize == 12 + 9 * geo.levels as usize,
        "mode geometry incoherent: {geo:?}"
    );
    // lower = canonical(1 << split_bit)，处内核半区
    assert_eq!(
        lower.as_usize(),
        (1usize << split) | (usize::MAX << (split + 1)),
        "lower not canonical kernel base"
    );
    assert!(!lower.is_user());
    // upper = 用户空间上界（1 << split_bit），排他边界（本身处规范空洞，非用户地址）；
    // 栈窗顶锚 [upper − STACK, upper)，窗口内地址须全部落在用户半区。
    assert_eq!(upper.as_usize(), top, "upper must equal user space ceiling");
    assert!(upper.as_usize().is_multiple_of(PAGE_SIZE));
    let stack_bottom = upper.as_usize() - STACK_WINDOW_SIZE;
    assert!(stack_bottom.is_multiple_of(PAGE_SIZE)); // 栈窗底对齐
    assert!(stack_bottom < upper.as_usize()); // 栈窗非空
    assert!(VirtAddr::wrap(stack_bottom).is_user()); // 栈窗内地址处用户半区
    // 布局常量在全部受支持模式下规范（wrap 前置条件的运行期印证）
    assert!(!TRAMPOLINE.is_user());
    assert!(HART_FRAME_BASE.as_usize() < TRAMPOLINE.as_usize());
    assert!(!TEAM_FRAME_BASE.is_user());
    // trap 栈窗口：kernel 半区（split 之上）+ 窗口恰止于 TEAM_FRAME_BASE
    assert!(!TRAP_STACK_BASE.is_user());
    assert!(
        TRAP_STACK_BASE.as_usize() + (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT)
            == TEAM_FRAME_BASE.as_usize()
    );
}
