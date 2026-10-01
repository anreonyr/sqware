use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
#[cfg(debug_assertions)]
use crate::memory::manager::mode;

/// **每一个任务那一叠用户栈的大小**（栈体，不含下面那一页保护页 —— 见
/// [`TASK_STACK_GUARD`]）。
///
/// **（32 KB → 64 KB：debug 档量出来的）**：debug 档的帧比 release 大好几倍（不内联、
/// 每个临时各占一格），编排域那条链
/// `main → system → Assembly::assemble → Control::enroll → Machine::devices`
/// 在 debug 档深到 32 KB 装不下。**实测**（`product` 景、debug 档、tid=11 = 编排域）：
///
///   · 折点：`Machine::devices` 第一句 `addi sp, sp, -0x6f0` 落下时 sp 已经在栈底以下
///     `0x408`（那台程序的 `image_end` = `0x56000` ⇒ 保护页 `[0x56000,0x57000)`、栈体
///     `[0x57000,0x5f000)`、起初 sp = `0x5f000`，此刻 sp = `0x56bf8`）；
///   · 下一句 `sd a1, 0x18(sp)` 存到 `0x56c10`，那正是保护页 ⇒ 内核按规矩报
///     `reserved region access: Store at VA(0x56c10), pc=0x25118` 并杀掉这一台。
///     **故障地址与"是哪一台"对得死**：`prog-system` 的 `image_end` 恰好是保护页的起点。
///   · 也就是说：这一条**不是内核的错** —— 保护页把 debug 档的栈饥饿照出来了。release 档
///     同一条链远在 32 KB 之内，故这一格一直没露。
///
/// **取值：两档同一个数（64 KB）**。按档分两值（debug 大 / release 小）会把"同一份代码两种
/// 内存布局"这种岔子引进来，而这一格只值 32 KB × 活着的任务数（本机峰值 ~20 台 ≈ 1.3 MB /
/// 256 MB）。64 KB 也与内核自己那叠启动栈（[`ROOT_STACK_SIZE`]）同数。
///
/// **改完量到的界**：64 KB 下 debug 档**不再折在保护页上**（一路走到装配后半：hub 落完
/// 二十格、router / uart / rtc 上树、`canonical` 印出用法行、`hole: live=0`）；release 档
/// `product` 景照旧全绿（用法行在、`exit tid=… system: done`、`hole: live=0`）。
///
/// **如实记（这一格修的不是 debug 档的全部）**：折掉保护页那一折之后，debug 档的 `product`
/// 景**仍会折**——折在装配期的树操作上，而且**victim 随镜像布局变**（实测三种构建各折一处：
/// `coalition: no identity plate` / `hub: no league plate` / `hub: tree`；另有一跑根本不折而在
/// `router: tree: road` 断言、随后 `mail: hand stuck hole#252 … age=2037ms` 卡住）。那一串与
/// release 档在案的残余同族（树那台单线程、客人有界等待被排在别人后头），**不是这一格能治的**
/// ——本条只保证"栈不是那一串的原因"。
pub(crate) const TASK_STACK_SIZE: usize = 64 * 1024;
pub(crate) const TASK_STACK_GUARD: usize = PAGE_SIZE;
pub(crate) const ROOT_STACK_SIZE: usize = 0x1_0000;

pub const MAX_HART_SLOTS: usize = 4096;

pub(crate) const TRAMPOLINE: VirtAddr = VirtAddr::wrap(0xFFFF_FFFF_FFFF_F000);

pub(crate) const KERNEL_TOP: VirtAddr =
    VirtAddr::wrap(TRAMPOLINE.as_usize() - (2 * 1024 * 1024 - PAGE_SIZE));

pub(crate) fn trampoline_pa() -> PhysAddr {
    unsafe extern "C" {
        static __trampoline_start: u8;
    }
    PhysAddr::from_raw(core::ptr::addr_of!(__trampoline_start) as usize)
}

pub(crate) const HART_FRAME_SLOTS: usize = MAX_HART_SLOTS;
pub(crate) const HART_FRAME_BASE: VirtAddr =
    VirtAddr::wrap(KERNEL_TOP.as_usize() - HART_FRAME_SLOTS * PAGE_SIZE);
pub(crate) const TEAM_FRAME_WINDOW_SIZE: usize = 64 * 1024 * 1024;
pub(crate) const TEAM_FRAME_BASE: VirtAddr =
    VirtAddr::wrap(HART_FRAME_BASE.as_usize() - TEAM_FRAME_WINDOW_SIZE);

pub(crate) const TRAP_STACK_SLOT_SIZE: usize = 64 * 1024;
pub(crate) const TRAP_STACK_SLOT_SHIFT: usize = 16;
pub(crate) const TRAP_STACK_GUARD: usize = PAGE_SIZE;
pub(crate) const TRAP_STACK_BASE: VirtAddr =
    VirtAddr::wrap(TEAM_FRAME_BASE.as_usize() - (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT));

pub const STACK_WINDOW_SIZE: usize = 0x4000_0000;
pub const IMAGE_BASE: VirtAddr = VirtAddr::wrap(0x1_0000);

const _: () = {
    assert!(TRAMPOLINE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(HART_FRAME_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(KERNEL_TOP.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TRAMPOLINE.as_usize() - KERNEL_TOP.as_usize() == 2 * 1024 * 1024 - PAGE_SIZE);
    assert!(HART_FRAME_BASE.as_usize() + HART_FRAME_SLOTS * PAGE_SIZE == KERNEL_TOP.as_usize());
    assert!(HART_FRAME_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_WINDOW_SIZE.is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE == HART_FRAME_BASE.as_usize());
    assert!(TASK_STACK_SIZE.is_multiple_of(PAGE_SIZE));
    assert!(TRAP_STACK_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(
        TRAP_STACK_BASE.as_usize() + (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT)
            == TEAM_FRAME_BASE.as_usize()
    );
    assert!(TRAP_STACK_SLOT_SIZE == 1usize << TRAP_STACK_SLOT_SHIFT);
    assert!(TRAP_STACK_GUARD == PAGE_SIZE);
};

#[cfg(debug_assertions)]
pub(crate) fn validate() {
    let geo = mode::geometry(mode::mode());
    let split = geo.split_bit() as usize;
    let top = 1usize << split;
    let lower = mode::lower();
    let upper = mode::upper();
    assert!(
        (3..=5).contains(&geo.levels) && geo.va_bits as usize == 12 + 9 * geo.levels as usize,
        "mode geometry incoherent: {geo:?}"
    );
    assert_eq!(
        lower.as_usize(),
        (1usize << split) | (usize::MAX << (split + 1)),
        "lower not canonical kernel base"
    );
    assert!(!lower.is_user());
    assert_eq!(upper.as_usize(), top, "upper must equal user space ceiling");
    assert!(upper.as_usize().is_multiple_of(PAGE_SIZE));
    let stack_bottom = upper.as_usize() - STACK_WINDOW_SIZE;
    assert!(stack_bottom.is_multiple_of(PAGE_SIZE));
    assert!(stack_bottom < upper.as_usize());
    assert!(VirtAddr::wrap(stack_bottom).is_user());
    assert!(!TRAMPOLINE.is_user());
    assert!(HART_FRAME_BASE.as_usize() < TRAMPOLINE.as_usize());
    assert!(!TEAM_FRAME_BASE.is_user());
    assert!(!TRAP_STACK_BASE.is_user());
    assert!(
        TRAP_STACK_BASE.as_usize() + (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT)
            == TEAM_FRAME_BASE.as_usize()
    );
}

/// **内核自己那叠启动栈**的哨兵（hart 0 起机时用的）。**它跟"引导镜像"无关**：那两个名字里的
/// `root` 说的是"任务树 / 启动那一下的根"，不是任何一个域的名字——**别再按域去读它**。
pub(crate) const ROOT_STACK_CANARY: usize = 0x600D_CAFE_51A7_0D1E;

pub(crate) fn kernel_edge() -> usize {
    (&raw const _kernel_edge).addr()
}
unsafe extern "C" {
    static _kernel_edge: u8;
}

#[unsafe(no_mangle)]
static _stack: usize = ROOT_STACK_SIZE;

#[unsafe(no_mangle)]
static _canary: usize = ROOT_STACK_CANARY;

pub(crate) fn root_stack_base() -> usize {
    kernel_edge()
}

pub(crate) fn root_stack_edge() -> usize {
    kernel_edge() + ROOT_STACK_SIZE
}
