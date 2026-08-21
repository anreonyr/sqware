// 陷阱处理 — stvec 接线、内核帧元数据、trap 分发（trap 运行时的 Rust 侧）
//
// 职责：
//   init()          — 写内核 trap-context 帧元数据、per-hart trap 栈 canary、
//                     stvec → __alltraps、sscratch 清 0、开 SIE
//   trap_handler()  — 汇编入口的 Rust 分发（scause 解码，类型化枚举），返回待恢复帧
//                     （当前任务续跑时恒为入参帧；切换时返回下一任务帧）
//   arm_timer()     — SBI set_timer 武装 S-timer 中断
//
// 内核态陷阱约定：现场保存在 per-hart 内核帧（KERNEL_FRAME_BASE + hart·PAGE），处理器运行在 per-hart
// trap 栈上；入口硬件已清 SIE，处理器内嵌套陷阱仅可能是内核 bug，会覆写内核
// 帧（panic 兜底）。trap 栈底 canary 在处理器出入口校验（溢出即 panic）。

use crate::runtime::{clock, timer};
use crate::work::room::scheduler::{run, unpark, with_running_space};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use core::fmt::Write;
use crate::console::Sink;
use table::Table;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sie, sip, sstatus, stval, stvec, time};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trace::{self, BootEvent, EventKind, MemEvent};
use crate::runtime::trampoline::{
    __trampoline_end, __trampoline_start, alltraps_va, establish_tp, init_trap_stacks,
    trap_stack_bottom, trap_stack_guard_hart, trap_stack_top,
};
use crate::work::unit::space::{KERNEL_FRAME_BASE, kernel_frame_pa};
use crate::{machine, put};
use sbi::{TimerCall, fid::Timer, scall::SArgs};

/// per-hart trap 栈底 canary（溢出检测：破坏即 panic；trampoline 的
/// init_trap_stacks 在 boot 时写全部 hart 的 canary）。
pub(crate) const TRAP_STACK_CANARY: usize = 0x5EED_CAFE_51A7_0000;

// 定时器周期 = 100 ms 抢占量子（由 clock 按 timebase 换算成 ticks）。
/// trap 栈已用字节数（高水位跟踪用）。
fn trap_stack_used() -> usize {
    let sp: usize;
    // SAFETY: 读当前栈指针，纯读无副作用。
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    trap_stack_top(machine::hart_id()).saturating_sub(sp)
}

/// trap 栈高水位（字节）——新峰值时打印一次（上限若干次，避免刷屏）。
static TRAP_STACK_PEAK: AtomicUsize = AtomicUsize::new(0);
static PEAK_PRINTS: AtomicUsize = AtomicUsize::new(0);

/// 更新高水位，新峰值且打印配额内时输出。
fn track_trap_stack_usage() {
    let used = trap_stack_used();
    let mut peak = TRAP_STACK_PEAK.load(Ordering::Relaxed);
    while used > peak {
        match TRAP_STACK_PEAK.compare_exchange_weak(
            peak,
            used,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                if PEAK_PRINTS.fetch_add(1, Ordering::Relaxed) < 8 {
                    trace::note(EventKind::Boot(BootEvent::Stack {
                        hart: machine::hart_id(),
                        used,
                    }));
                }
                break;
            }
            Err(current) => peak = current,
        }
    }
}

/// 初始化 trap 运行时（须在 `unit::init` 之后：内核帧与 TRAMPOLINE 映射已就绪）。
pub fn init() {
    // 0. per-hart trap 栈：frame 连续分配 + guard 页 + 全部 canary（先于内核帧
    //    元数据——帧 kernel_sp 需要指向本 hart 栈顶）。仅 hart 0 调用一次。
    init_trap_stacks();

    // 1. 防呆：trampoline 汇编必须落在一页内（TRAMPOLINE 映射只覆盖一页）
    let tsize = (core::ptr::addr_of!(__trampoline_end) as usize)
        - (core::ptr::addr_of!(__trampoline_start) as usize);
    assert!(
        tsize <= PAGE_SIZE,
        "trampoline exceeds one page: {tsize:#x}"
    );

    // 2. per-hart 内核 trap-context 帧元数据（帧由 unit::init 逐页映射，PA 已
    //    发布）。B2：每 hart 一份——kernel_sp = 本 hart trap 栈顶，__strap 按 TP
    //    索引帧页；内核态故障在**故障核**的帧与 trap 栈上处理。用户帧的
    //    kernel_sp 由调度器每切换写入（见 scheduler::prepare_resume）。
    let ksatp = satp::read();
    for h in 0..crate::machine::hart_count() {
        let pa = kernel_frame_pa(h);
        let frame = unsafe { &mut *(pa.as_usize() as *mut TrapContext) };
        frame.kernel_satp = ksatp;
        frame.kernel_sp = VirtAddr::from_raw(trap_stack_top(h));
        frame.trap_handler = VirtAddr::from_raw(trap_handler as *const () as usize);
        frame.trap_stack_corrupt = TRAP_STACK_CANARY;
        frame.user_pa = pa;
        frame.user_satp = ksatp;
        // self_va：本 hart 内核帧 VA（restore 切表后经此收尾）
        frame.self_va = KERNEL_FRAME_BASE + h * PAGE_SIZE;
    }

    // 3. 先武装定时器：OpenSBI 可能遗留一个已到期的 stimecmp，若不清掉，
    //    开中断瞬间会立即触发一次 S-timer 陷阱（无害但时序难看）。
    arm_timer(clock::duration_to_ticks(Duration::from_millis(100)));

    // 4. stvec → __alltraps（Direct 模式）；sscratch = 0（内核态约定）；
    //    使能定时器源：sie.STIE。**不**开 sstatus.SIE（全局）——内核态恒关中断
    //    （处理器内关中断策略）：SIE 只经 sret 由帧内 SPIE 恢复，用户态 = 1，
    //    内核态 = 0。故 S-timer 只在用户态触发，内核代码永不被打断/抢占。
    //    （本步是 hart 0 的 CSR；副核经 init_hart 各自配置。）
    unsafe {
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        core::arch::asm!("csrw sscratch, zero");
        sie::set_stimer();
        sie::set_ssoft(); // SSIP 使能：WFI 休眠核被 SBI IPI 唤醒的前提（只唤醒不取中断）
    }

    let mut l = Table::<4, 2, 96>::new();
    l.cell(0, 0).push_str("trap vector");
    let _ = write!(l.cell(0, 1), "{:#x}", alltraps_va());
    l.cell(1, 0).push_str("kernel frames");
    let _ = write!(l.cell(1, 1), "{:#x}..{:#x}", KERNEL_FRAME_BASE.as_usize(), KERNEL_FRAME_BASE.as_usize() + crate::machine::hart_count() * PAGE_SIZE);
    l.cell(2, 0).push_str("trap stack");
    let _ = write!(l.cell(2, 1), "{:#x}..{:#x}", trap_stack_bottom(0), trap_stack_top(0));
    l.cell(3, 0).push_str("trap stack this");
    let _ = write!(l.cell(3, 1), "{} @ {:#x}..{:#x}", machine::hart_id(), trap_stack_bottom(machine::hart_id()), trap_stack_top(machine::hart_id()));
    let mut sink = Sink;
    let _ = l.render(&mut sink);
}

/// 武装 S-timer 中断：`stimecmp = time + interval`（SBI TIME 扩展，绝对时间）。
/// interval 为 timebase 刻度（clock 换算）；休眠推远见 scheduler::wait。
pub fn arm_timer(interval: u64) {
    let next = (time::read() as u64).wrapping_add(interval);
    TimerCall::new(Timer::SetTimer)
        .args(SArgs {
            a0: next as usize,
            ..Default::default()
        })
        .call()
        .unwrap();
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
    // 0. 重建内核 tp（= hartid）：用户态可能改写过 tp；一切 hart_id() 依赖它。
    //    由当前 sp（trap 栈体内）反解段号（trampoline::establish_tp）。
    establish_tp();

    // 0.5 多核 panic：警报已拉响且本 hart 非报警源 → 就地卧倒（不返回）。
    //    用户态核的定时器/IPI 陷阱、及任何 trap 入口都会在此拦下并自停；
    //    正常运行时恒 no-op。
    crate::runtime::halt::hush();

    // 1. trap 栈 guard 溢出特判（先于 canary：溢出可能已破坏 canary 字）。
    //    仅缺页类 scause 才读 stval（其余陷阱 stval 无意义，可能残留旧值）。
    let cause = scause::read();
    if cause.is_exception() && matches!(cause.code(), 12 | 13 | 15) {
        let stv = stval::read();
        if let Some(h) = trap_stack_guard_hart(stv) {
            panic!("trap stack overflow on hart {h} (stval = {stv:#x})");
        }
    }

    // 1. 入口校验：per-hart trap 栈 canary 与内核帧标记（上一次处理器若溢出，
    //    此处立即暴露——canary 由 init_trap_stacks 写在每段栈底）
    let me = machine::hart_id();
    let canary = unsafe { (trap_stack_bottom(me) as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} (overflow?)"
    );
    assert_eq!(
        frame.trap_stack_corrupt, TRAP_STACK_CANARY,
        "kernel trap frame corrupted"
    );
    // 2. debug：用户态陷阱必须运行在当前 hart 的 trap 栈上（kernel_sp 每切换
    //    写入的正确性——steal 迁移后写漏即在此暴露；内核态故障走共享内核帧 +
    //    hart 0 栈，跳过本检查）。
    #[cfg(debug_assertions)]
    if frame.sstatus.spp() != sstatus::SPP::Supervisor {
        let sp: usize;
        // SAFETY: 读当前栈指针，纯读无副作用。
        unsafe {
            core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
        }
        let top = trap_stack_top(me);
        let ksp = frame.kernel_sp.as_usize();
        let tid = crate::work::room::scheduler::running_task_id();
        debug_assert!(
            sp <= top && top - sp < 0x4000,
            "user trap on hart {me}: sp={sp:#x} top={top:#x} frame.kernel_sp={ksp:#x} (task #{tid}) — kernel_sp per-switch write missing?"
        );
    }
    track_trap_stack_usage();

    // 类型化分发：裸码 → riscv::interrupt 枚举（try_into 对标准集外码返回 Err，
    // 不会 panic；Err 分支给出诊断）。变体即规范语义：SupervisorTimer=5、
    // UserEnvCall=8、InstructionPageFault=12、LoadPageFault=13、StorePageFault=15。
    let trap: Trap<Interrupt, Exception> = scause::read().cause().try_into().unwrap_or_else(|e| {
        panic!("unknown trap cause: {e:?}");
    });
    let next: *mut TrapContext = match trap {
        // S-timer：重武装 + 抢占（仅用户态陷阱可切换——内核态陷阱须恢复被
        // 中断的内核上下文；内核恒关中断下本不应发生，SPP 判断为防御性）
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            timer::tick();
            // 重武装：运行任务抢占量子；到期唤醒由 unpark 经 timer::drain 驱动
            arm_timer(clock::duration_to_ticks(Duration::from_millis(100)));
            unpark();
            // 值班看护：健康核打点报岗 + 巡岗查险（失速/静默/锁相持）——这里是
            // 每量子的定时器节拍，是 watchdog 判据的可靠驱动点。
            crate::work::room::scheduler::round();
            if frame.sstatus.spp() != sstatus::SPP::Supervisor {
                run() as *mut TrapContext
            } else {
                frame as *mut TrapContext
            }
        }
        Trap::Interrupt(Interrupt::SupervisorSoft) => {
            // IPI 唤醒信号。wake_all 按 WAITING 位图发 IPI；竞态窗口：IPI 发出时
            // 目标核可能已离开 WFI（steal 取走任务、回到用户态 SIE=1）→ SSIP 挂起
            // 会被真取中断。业务（steal 复查）在 wait()/wake_all 路径已闭合；此处
            // 是最后一站：清挂起位（不清则 sret 后立即再取 → 中断风暴），静默放行。
            unsafe {
                sip::clear_ssoft();
            }
            frame as *mut TrapContext
        }
        Trap::Interrupt(other) => {
            put!("unhandled interrupt: {other:?}\n{frame:#?}\n");
            frame as *mut TrapContext
        }
        // 用户态环境调用（U 态 ecall）：envcall 表分发
        Trap::Exception(Exception::UserEnvCall) => crate::runtime::envcall::dispatch(frame),
        // 用户态缺页：机制归 memory::fault，策略归 trap 层（解析失败即 panic）。
        // SPP=1（内核态）缺页：guard 已在入口特判，其余内核缺页 = 内核 bug → fatal
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
            let ok = with_running_space(|space| {
                crate::memory::manager::fault::handle_page_fault(&fault, space)
            });
            trace::note(EventKind::Mem(MemEvent::PageFault {
                va: fault.addr.as_usize(),
                fault: fault.kind,
                resolved: ok,
            }));
            if ok {
                putln!("user page fault resolved: {fault:?}");
            } else {
                panic!("unresolved page fault: {fault:?}");
            }
            frame as *mut TrapContext
        }
        // 其余异常（含 S-mode envcall = 内核 bug）：fatal
        Trap::Exception(other) => {
            panic!(
                "unhandled kernel exception: {other:?} at sepc={:#x}, stval={:#x}",
                sepc::read(),
                stval::read()
            );
        }
    };

    // 出口再校验一次 canary（处理器自身栈用量引发的溢出）
    let me = machine::hart_id();
    let canary = unsafe { (trap_stack_bottom(me) as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} after handler"
    );

    next
}