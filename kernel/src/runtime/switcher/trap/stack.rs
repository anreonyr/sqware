//! trap 栈 — per-hart 固定 VA 窗口与它的 O(1) 反解（本模块零表、零堆依赖）。
//!
//! 布局（`layout::TRAP_STACK_*`）：`TRAP_STACK_BASE` 起，hart h 段 = BASE + h·64 KiB：
//! 首页 guard（内核空间未映射，越界即页故障）、其下 60 KiB 栈体（boot 时映射物理页）。
//! 反解：段大小 = 2^SHIFT ⇒ hart = (sp − BASE) >> SHIFT——零元数据表，堆破坏不再
//! 能经表污染 hart 判定。崩溃路径（scene 钳制、guard 识别）与正常路径同源。
//!
//! 符号经 `trap` 门面转出（`switcher::trap::trap_stack_edge` 等），公开路径不变。

use core::time::Duration;

use riscv::register::{satp, sie, stvec};

use crate::layout::{
    HART_FRAME_BASE, TRAP_STACK_BASE, TRAP_STACK_GUARD, TRAP_STACK_SLOT_SHIFT, TRAP_STACK_SLOT_SIZE,
};
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, check_fits_page};
use crate::work::unit::team::kernel;

/// per-hart trap 栈底 canary（溢出检测：破坏即 panic；boot 时写入全部 hart）。
pub(crate) const TRAP_STACK_CANARY: usize = 0x5EED_CAFE_51A7_0000;

// ── per-hart trap 栈：固定 VA 窗口 + 纯算术反解（无元数据表）──
//
// 布局（layout::TRAP_STACK_*）：TRAP_STACK_BASE 起，hart h 段 =
//   TRAP_STACK_BASE + h·64 KiB：首页 guard（内核空间未映射，越界即页故障）、
//   其下 60 KiB 栈体（boot 时映射物理页）。
// 反解：段大小 = 2^SHIFT ⇒ hart = (sp − BASE) >> SHIFT（O(1)、零表、零堆依赖——
//   堆破坏不再能经元数据表污染 hart 判定）。崩溃路径（scene 钳制、guard 识别）
//   与正常路径同源，均不依赖任何运行时表。

/// trap 栈物理块基址（boot 时 frame 连续分配 N×64 KiB，段物理首址 = base + h·SEGMENT）。
/// 仅副核 HSM 启动栈（bare 模式，sp 必须是物理地址）使用；trap 侧一律走固定 VA。
static TRAP_STACK_PHYS: OnceLock<usize> = OnceLock::new();

/// hart h 的 trap 栈段几何（纯算术）：守卫页不映射，栈体 = [base, edge)。
/// base = 段首 + guard（canary 处）；edge = 段上界（排他，初始 sp 落点）。
fn trap_stack_segment(hart: usize) -> (VirtAddr, VirtAddr) {
    let base = TRAP_STACK_BASE.as_usize() + hart * TRAP_STACK_SLOT_SIZE;
    (
        VirtAddr::from_raw(base + TRAP_STACK_GUARD),
        VirtAddr::from_raw(base + TRAP_STACK_SLOT_SIZE),
    )
}

/// hart 的 trap 栈体底（固定 VA，canary 处）。
pub fn trap_stack_base(hart: usize) -> VirtAddr {
    trap_stack_segment(hart).0
}

/// hart 的 trap 栈体上边界（固定 VA，排他端；初始 sp 落点）。
pub fn trap_stack_edge(hart: usize) -> VirtAddr {
    trap_stack_segment(hart).1
}

/// sp 是否落在某 hart 的 trap 栈体内（guard 之上、edge 之下→含）——反解 hart。
///
/// 崩溃路径的瘦身版 `establish_tp`：推 hart 不读表、不 panic；越出窗口/guard/
/// 未启用核一律 None（引导期与非法现场合法返回）。正常路径恒命中：trap handler
/// 恒在 per-hart trap 栈上执行。
pub(crate) fn trap_stack_hart(sp: usize) -> Option<usize> {
    let off = sp.checked_sub(TRAP_STACK_BASE.as_usize())?;
    let h = off >> TRAP_STACK_SLOT_SHIFT;
    if h >= crate::machine::hart_count() {
        return None;
    }
    let in_seg = off & (TRAP_STACK_SLOT_SIZE - 1);
    (in_seg > TRAP_STACK_GUARD && in_seg <= TRAP_STACK_SLOT_SIZE).then_some(h)
}

/// 地址是否落在某 hart 的 trap 栈 guard 页内（返回该 hart 号）——内核故障
/// 路径据此识别「trap 栈溢出」并给出精确诊断。纯算术，不读表。
pub(crate) fn trap_stack_guard_hart(addr: usize) -> Option<usize> {
    let off = addr.checked_sub(TRAP_STACK_BASE.as_usize())?;
    if off & (TRAP_STACK_SLOT_SIZE - 1) < TRAP_STACK_GUARD {
        let h = off >> TRAP_STACK_SLOT_SHIFT;
        (h < crate::machine::hart_count()).then_some(h)
    } else {
        None
    }
}

/// trap 栈物理块基址（`init` 的装配产物）。仅 boot_harts 组装
/// HSM opaque（副核启动栈物理栈顶 = base + (h+1)·SEGMENT）使用。
pub fn trap_stack() -> usize {
    *TRAP_STACK_PHYS.get().expect("trap stacks not initialized")
}

/// 初始化 trap 运行时（须在 `unit::init` 之后：hart 帧与 TRAMPOLINE 映射已就绪）。
pub fn init() {
    // 0. 物理支撑校验：per-hart 固定开销（trap 栈段 64 KiB + hart 帧页 4 KiB）
    //    必须不超出 free 物理池——「内存制约最大核数」的运行时落点（编译期
    //    MAX_HART_SLOTS 只是 VA 布局表达上限，物理养活上限由本校验把握）。
    let per_hart = TRAP_STACK_SLOT_SIZE + PAGE_SIZE;
    let need = crate::machine::hart_count() * per_hart;
    assert!(
        crate::machine::info().free.size >= need,
        "hart_count {} needs {need:#x} B ({}×{per_hart:#x}) but free pool is {:#x} B",
        crate::machine::hart_count(),
        crate::machine::hart_count(),
        crate::machine::info().free.size,
    );

    // 1. per-hart trap 栈：frame 连续分配 + guard 页 + 全部 canary（先于 hart 帧
    //    元数据——帧 kernel_sp 需要指向本 hart 栈顶）。仅 hart 0 调用一次。
    let segments = crate::machine::hart_count();
    assert!(segments > 0, "no harts");
    assert_eq!(
        TRAP_STACK_SLOT_SIZE,
        1 << TRAP_STACK_SLOT_SHIFT,
        "trap stack segment must be 2^SHIFT"
    );
    let total = segments * TRAP_STACK_SLOT_SIZE;
    let layout = core::alloc::Layout::from_size_align(total, PAGE_SIZE).expect("trap stack layout");
    // 块连续（frame 按 order 取整到 2 的幂）；boot 期帧池充足。种类 = TrapStack
    // （boot 持久帧——装饰器标注，种类记账收在 fence）。
    let block = crate::tag!(
        TrapStack,
        crate::memory::allocator::frame::allocator()
            .allocate(layout)
            .expect("trap stack block allocation")
    );
    let base = block.cast::<u8>().as_ptr() as usize;
    // 持久注册表：trap 栈块永不归还——登记以便关机逐项核 held（Held 组）。
    #[cfg(feature = "audit")]
    crate::memory::allocator::fence::audit::register_persistent(
        base,
        crate::memory::allocator::fence::Kind::TrapStack,
    );
    assert!(
        TRAP_STACK_PHYS.set(base).is_ok(),
        "trap stack phys double init"
    );

    let space = &kernel().expect("kernel team not initialized").space;
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    for h in 0..segments {
        let (body_va, _edge) = trap_stack_segment(h);
        let phys = base + h * TRAP_STACK_SLOT_SIZE;
        // 段体映射（60 KiB）：固定 VA → 块内物理页；guard 页不映射（越界即页故障）
        space
            .borrow(
                body_va,
                PhysAddr::from_raw(phys + TRAP_STACK_GUARD),
                TRAP_STACK_SLOT_SIZE - TRAP_STACK_GUARD,
                flags,
            )
            .expect("map trap stack body");
        // 恒等视图 guard 页清 PTE 保留 boot 栈溢出护栏（固定 VA guard 管 trap 栈）
        space.unmap(VirtAddr::from_raw(phys), TRAP_STACK_GUARD);
        // canary 写于固定 VA 栈体底（guard 之上）
        unsafe {
            (body_va.as_usize() as *mut usize).write(TRAP_STACK_CANARY);
        }
    }

    // 2. 防呆：trampoline 汇编必须落在一页内（TRAMPOLINE 映射只覆盖一页）
    check_fits_page();

    // 3. per-hart trap-context 帧元数据（帧已逐页映射，PA 已发布）。每 hart
    //    一份——kernel_sp = 本 hart trap 栈顶，__strap 按 TP 索引帧页；内核态
    //    故障在**故障核**的帧与 trap 栈上处理。
    let ksatp = satp::read();
    for h in 0..crate::machine::hart_count() {
        let pa = kernel()
            .expect("kernel team not initialized")
            .space
            .translate(HART_FRAME_BASE + h * PAGE_SIZE)
            .expect("kernel frame not mapped")
            .0;
        let frame = unsafe { &mut *(pa.as_usize() as *mut TrapContext) };
        frame.kernel_satp = ksatp;
        frame.kernel_sp = trap_stack_edge(h);
        frame.trap_handler = VirtAddr::from_raw(super::trap_handler as *const () as usize);
        frame.trap_stack_corrupt = TRAP_STACK_CANARY;
        frame.user_pa = pa;
        frame.user_satp = ksatp;
        // self_va：本 hart 帧 VA（restore 切表后经此收尾）
        frame.self_va = HART_FRAME_BASE + h * PAGE_SIZE;
    }

    // 4. 先武装定时器：OpenSBI 可能遗留一个已到期的 stimecmp，若不清掉，
    //    开中断瞬间会立即触发一次 S-timer 陷阱（无害但时序难看）。
    timer::beat(clock::duration_to_ticks(Duration::from_millis(100)));

    arm_hart();
}

/// 武装**当前执行 hart** 的 trap 运行时：stvec → trap 入口（Direct）、sscratch →
/// 本 hart 帧 VA（内核态约定）、sie 开 STIE + SSIE。
///
/// 前置：本 hart 帧元数据已填（`init` 装配后）；stvec 目标 = 已映射的 TRAMPOLINE 页。
/// 调用方：hart 0 由 `init()` 调；副核由 `boot_main` 在切 satp 后调——同一原语。
pub fn arm_hart() {
    unsafe {
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        // PerHart.frame 经 tp 直达（执行核帧 VA；与 __strap 帧定位同源）。
        let scr = crate::machine::hart_frame().as_usize();
        core::arch::asm!("csrw sscratch, {}", in(reg) scr);
        sie::set_stimer();
        sie::set_ssoft(); // SSIP 使能：WFI 休眠核被 SBI IPI 唤醒的前提（只唤醒不取中断）
    }
}
