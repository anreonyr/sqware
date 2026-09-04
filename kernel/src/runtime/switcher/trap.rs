// 陷阱处理 — trap 运行时的 Rust 侧核心 + per-hart 上下文（切换机械 / 栈几何 / 装配）
//
// 三条线在此收口：
// 1. 分发核心：`trap_handler`（tp 重建、guard 特判、canary 校验、类型化分发、
//    被抢占内核现场持久化）——汇编入口在 `trampoline`（帧 ABI 见 `context.rs`）。
// 2. per-hart trap 栈几何（纯算术反解，无元数据表）：base/edge/hart/guard ——
//    入口（tp 重建前）按 sp 反解 hart；崩溃路径（scene 钳制、guard 识别）同源。
// 3. 装配（适配层：boot）：`init` —— trap 栈块分配 + 页防呆 + 帧元数据 + 定时器
//    + `arm_hart`（per-hart CSR 接线，hart 0 与副核共用同一原语）。
//
// sscratch 约定（与 trampoline/__restore 一致）：用户态 = 当前线程帧 VA；内核态 =
// 本 hart 帧 VA（HART_FRAME_BASE + hart·PAGE）——arm_hart（hart 0/副核统一接线）
// 与 __restore SPP=1 维护。崩溃场景（scene.rs）按值域判定并可直接反推 hart。
//
// 内核态陷阱约定：现场保存在 hart 帧（HART_FRAME_BASE + hart·PAGE），处理器运行在
// per-hart trap 栈上；入口硬件已清 SIE，处理器内嵌套陷阱仅可能是内核 bug，会覆写
// per-hart 帧（panic 兜底）。trap 栈底 canary 在处理器出入口校验（溢出即 panic）。

use core::time::Duration;

use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sie, sip, sstatus, stval, stvec};

use crate::layout::{
    HART_FRAME_BASE, TRAP_STACK_BASE, TRAP_STACK_GUARD, TRAP_STACK_SLOT_SHIFT, TRAP_STACK_SLOT_SIZE,
};
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::evict;
use crate::putln;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, MemoryEvent, RoomEvent};
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, check_fits_page};
use crate::work::room::messenger::drain_expired;
use crate::work::room::scheduler::core::{Current, ident};
use crate::work::room::scheduler::trap::run;
use crate::work::unit::space::SpaceKind;
use crate::work::unit::team::kernel;
use crate::{machine, put};

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
fn trap_stack_guard_hart(addr: usize) -> Option<usize> {
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
    // 块连续（frame 按 order 取整到 2 的幂）；boot 期帧池充足。类别 = Persistent
    // （boot 持久帧——装饰器标注，类别记账收在 fence）。
    let block = crate::tag!(
        Persistent,
        crate::memory::allocator::frame::allocator()
            .allocate(layout)
            .expect("trap stack block allocation")
    );
    let base = block.cast::<u8>().as_ptr() as usize;
    // 持久注册表：trap 栈块永不归还——登记以便关机逐项核 held（②）。
    #[cfg(feature = "audit")]
    crate::memory::allocator::fence::audit::register_persistent(base, "trap-stack");
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
            .borrow_map(
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
        frame.trap_handler = VirtAddr::from_raw(trap_handler as *const () as usize);
        frame.trap_stack_corrupt = TRAP_STACK_CANARY;
        frame.user_pa = pa;
        frame.user_satp = ksatp;
        // self_va：本 hart 帧 VA（restore 切表后经此收尾）
        frame.self_va = HART_FRAME_BASE + h * PAGE_SIZE;
    }

    // 4. 先武装定时器：OpenSBI 可能遗留一个已到期的 stimecmp，若不清掉，
    //    开中断瞬间会立即触发一次 S-timer 陷阱（无害但时序难看）。
    timer::tick_after(clock::duration_to_ticks(Duration::from_millis(100)));

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

/// 内核态被打断的现场持久化：把 hart 帧（仅一份）的被中断现场
/// （gpr/sstatus/sepc）拷入当前 running 任务的专属帧。否则抢占切走后再来
/// 陷阱会覆写 hart 帧——被抢占内核任务的现场将丢失。
///
/// 判定源（与调度域的 D2-1 收敛一致）：「running 任务是内核任务」由任务所属
/// 空间的 kind 决定（不再读 sstatus.spp）。软陷阱（scheduler::ktask）与硬件
/// 抢占路径共用本搬移。
///
/// 自查询形态：身份经 `ident()` 无锁读槽（ktask 汇编消费者无法传参；槽读廉价、
/// 崩溃现场安全）。只搬三个现场字段；任务帧其余元数据（kernel_satp/kernel_sp/
/// trap_handler/user_satp/self_va/…）由 spawn/prepare 维护，不得改动。
pub(crate) fn persist(frame: &TrapContext) -> bool {
    let Some(i) = ident() else {
        return false;
    };
    // Live 轴才有地址空间与 trap 帧：S 态空闲（末次身份）时不得写入——帧可能
    // 已被 clear 归还重分配，写即覆写他人帧（评审漏掉的第二个悬垂点，写侧）。
    let Some(task) = i.live() else {
        return false;
    };
    if !matches!(task.team.space.kind(), SpaceKind::Kernel) {
        return false;
    }
    let Some(pa) = i.trap() else {
        return false;
    };
    let dst = pa.as_usize() as *mut TrapContext;
    // SAFETY: 任务专属帧 PA 恒等映射可写；当前 running 任务独占；此后不再使用
    // hart 帧（run() 切换返回下一任务帧，由 __restore 消耗）。
    unsafe {
        (*dst).gpr = frame.gpr;
        (*dst).sstatus = frame.sstatus;
        (*dst).sepc = frame.sepc;
    }
    true
}

/// 陷阱分发 — 汇编入口（`jalr trap_handler`）的唯一 Rust 侧。
///
/// 入参 `frame` = 被中断上下文的帧（汇编以 a0 = 帧物理地址调用，恒等映射下
/// 引用即物理地址）；返回值 = 待恢复帧（当前任务续跑时恒为入参帧；切换时
/// 返回下一任务帧）。
///
/// # Safety
///
/// 仅由 trampoline 汇编调用：入参必须指向有效且独占的 `TrapContext`（帧独占性
/// 由汇编入口/出口顺序保证——每次陷阱新建引用，无并发别名），且当前处于陷阱
/// 上下文（中断屏蔽、CSR 已由硬件保存）。
#[unsafe(no_mangle)]
pub(crate) extern "C" fn trap_handler(frame: &mut TrapContext) -> *mut TrapContext {
    // 0. 重建内核 tp（= 本 hart PerHart 指针）：用户态可能改写过 tp；一切
    //    hart_id() 依赖它。由当前 sp（trap 栈体内）反解段号（trap_stack_hart）。
    let sp: usize;
    // SAFETY: 读当前栈指针，纯读无副作用。
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    let hart = trap_stack_hart(sp).unwrap_or(0);
    let tp = crate::machine::per_hart_ptr(hart);
    // SAFETY: 写线程指针寄存器（仅 trap 入口调用一次，重建本 hart PerHart 指针）。
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) tp, options(nomem, nostack, preserves_flags));
    }

    // 0.4 入场入册：`__utrap`/`__strap` 已整表刷（不变量 1），本核转为内核租户。
    evict::settle(0);

    // 0.5 本核当前任务身份（None = 空闲/boot/早期 panic——各分支自行降级）。
    let ident = ident();

    // 0.6 多核 panic：警报已拉响且本 hart 非报警源 → 就地卧倒（不返回）；
    //    正常运行时恒 no-op。
    crate::runtime::diagnose::halt::hush();

    // 1. trap 栈 guard 溢出特判（先于 canary：溢出可能已破坏 canary 字）。
    //    仅缺页类 scause 才读 stval（其余陷阱 stval 无意义，可能残留旧值）。
    let cause = scause::read();
    if cause.is_exception() && matches!(cause.code(), 12 | 13 | 15) {
        let stv = stval::read();
        if let Some(h) = trap_stack_guard_hart(stv) {
            panic!("trap stack overflow on hart {h} (stval = {stv:#x})");
        }
    }

    // 1. 入口校验：per-hart trap 栈 canary 与 hart 帧标记（上一次处理器若溢出，
    //    此处立即暴露——canary 由 init 写在每段栈底）
    let me = machine::hart_id();
    let canary = unsafe { (trap_stack_base(me).as_usize() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} (overflow?)"
    );
    assert_eq!(
        frame.trap_stack_corrupt, TRAP_STACK_CANARY,
        "kernel trap frame corrupted"
    );
    // 2. debug：用户态陷阱必须运行在当前 hart 的 trap 栈上（kernel_sp 每次
    //    切换写入的正确性——任务迁移后写漏即在此暴露）。用户陷阱必有任务。
    #[cfg(debug_assertions)]
    if let Some(i) = ident.as_ref()
        && frame.sstatus.spp() != sstatus::SPP::Supervisor
    {
        let sp: usize;
        // SAFETY: 读当前栈指针，纯读无副作用。
        unsafe {
            core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
        }
        let top = trap_stack_edge(me).as_usize();
        let ksp = frame.kernel_sp.as_usize();
        debug_assert!(
            sp <= top && top - sp < 0x4000,
            "user trap on hart {me}: sp={sp:#x} top={top:#x} frame.kernel_sp={ksp:#x} (task #{}) — kernel_sp per-switch write missing?",
            i.id()
        );
    }

    // 类型化分发：裸码 → riscv::interrupt 枚举（try_into 对标准集外码返回 Err，
    // 不会 panic；Err 分支给出诊断）。变体即规范语义：SupervisorTimer=5、
    // UserEnvCall=8、InstructionPageFault=12、LoadPageFault=13、StorePageFault=15。
    let trap: Trap<Interrupt, Exception> = scause::read().cause().try_into().unwrap_or_else(|e| {
        panic!("unknown trap cause: {e:?}");
    });
    let next: *mut TrapContext = match trap {
        // S-timer：重武装 + 抢占。用户态陷阱直接切换（现场本就在任务帧）；
        // 内核态陷阱（可抢占内核）先把现场持久化到任务专属帧再切换——per-hart
        // 帧仅一份，不搬即被下一次 trap 覆写，被抢占内核任务现场丢失。
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            timer::tick();
            // 重武装：运行任务抢占量子。
            timer::tick_after(clock::duration_to_ticks(Duration::from_millis(100)));
            drain_expired();
            if frame.sstatus.spp() != sstatus::SPP::Supervisor {
                run() as *mut TrapContext
            } else if persist(frame) {
                // 内核态被打断且确有 running 内核任务：现场已持久化 → 抢占
                run() as *mut TrapContext
            } else {
                // S 态空闲（无 running：取活/WFI 被 timer 打断）→ 恢复原上下文
                frame as *mut TrapContext
            }
        }
        Trap::Interrupt(Interrupt::SupervisorSoft) => {
            // IPI 唤醒信号（SSIP）：清挂起位（不清则 sret 后立即再取 → 中断
            // 风暴），静默放行。
            unsafe {
                sip::clear_ssoft();
            }
            frame as *mut TrapContext
        }
        Trap::Interrupt(other) => {
            put!("unhandled interrupt: {other:?}\n{frame:#?}\n");
            frame as *mut TrapContext
        }
        // 用户态环境调用（U 态 ecall）：envcall 表分发（ecall 必有任务）。
        // 身份 Arc **移交**给 dispatch：其内部在可能触发 halt（run）的分支（Reap/
        // Park/Wait）先 drop——否则 halt 时本核 trap_handler 仍持最后任务的
        // Arc<TaskIdent> → team → space 被钉住不 drop，关机审计误报帧泄漏。
        Trap::Exception(Exception::UserEnvCall) => {
            let Some(Current::Live(ident_arc)) = ident else {
                panic!("envcall without running task");
            };
            crate::runtime::switcher::envcall::dispatch(frame, ident_arc)
        }
        // 用户态缺页：解析成功 → 续跑；解析失败 → fault isolation 杀 task。
        // SPP=Supervisor（内核态）缺页 = 内核 bug → 仍 panic。
        Trap::Exception(
            Exception::InstructionPageFault | Exception::LoadPageFault | Exception::StorePageFault,
        ) => {
            if frame.sstatus.spp() == sstatus::SPP::Supervisor {
                panic!(
                    "kernel page fault on hart {} at sepc={:#x}, stval={:#x}",
                    machine::hart_id(),
                    sepc::read(),
                    stval::read()
                );
            }
            let fault = unsafe { crate::memory::manager::fault::PageFault::capture() };
            let running = ident
                .as_ref()
                .and_then(Current::live)
                .expect("user page fault without running task");
            let ok = crate::memory::manager::fault::handle_page_fault(&fault, &running.team.space);
            trace::note(EventKind::Memory(MemoryEvent::PageFault {
                va: fault.addr.as_usize(),
                fault: fault.kind,
                resolved: ok,
            }));
            if ok {
                putln!("user page fault resolved: {fault:?}");
                return frame as *mut TrapContext;
            }
            // 不可解析 → 杀 task（不复用 frame：reap 取下一任务的 frame PA）。
            let tid = running.id;
            let cause_bits = scause::read().bits();
            let stval_bits = stval::read();
            trace::note(EventKind::Room(RoomEvent::FaultKilled {
                tid,
                cause: cause_bits,
                stval: stval_bits,
            }));
            putln!(
                "user fault killed: tid={tid} cause={cause_bits} stval={stval_bits:#x}"
            );
            drop(ident);
            return crate::work::room::scheduler::utask::reap() as *mut TrapContext;
        }
        // 异常：SPP=User → fault isolation 杀 task；SPP=Supervisor → 内核 bug → fatal。
        Trap::Exception(other) => {
            if frame.sstatus.spp() == sstatus::SPP::User {
                let running = ident
                    .as_ref()
                    .and_then(Current::live)
                    .expect("user exception without running task");
                let tid = running.id;
                let cause_bits = scause::read().bits();
                let stval_bits = stval::read();
                trace::note(EventKind::Room(RoomEvent::FaultKilled {
                    tid,
                    cause: cause_bits,
                    stval: stval_bits,
                }));
                putln!(
                    "user exception killed: tid={tid} cause={:?} stval={stval_bits:#x}",
                    other
                );
                drop(ident);
                return crate::work::room::scheduler::utask::reap() as *mut TrapContext;
            }
            panic!(
                "unhandled kernel exception: {other:?} at sepc={:#x}, stval={:#x}",
                sepc::read(),
                stval::read()
            );
        }
    };

    // 出口再校验一次 canary（处理器自身栈用量引发的溢出）
    let me = machine::hart_id();
    let canary = unsafe { (trap_stack_base(me).as_usize() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} after handler"
    );

    // 出场公布：租约取**将要返回的帧**（`run()` 可能已换任务），且必须在返回
    // 之前——不变量 2（先公布，后 `__restore` 的 sfence）。
    // SAFETY: next 恒指向本核有效帧（分发各分支的产物），恒等映射下可解引用。
    evict::settle(unsafe { (*next).user_satp.asid() });

    next
}
