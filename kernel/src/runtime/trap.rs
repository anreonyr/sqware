// 陷阱处理 — stvec 接线、内核帧元数据、trap 分发（trap 运行时的 Rust 侧）
//
// 职责：
//   init()          — 写内核 trap-context 帧元数据、per-hart trap 栈 canary、
//                     stvec → __alltraps、sscratch 清 0、开 SIE
//   trap_handler()  — 汇编入口的 Rust 分发（scause 解码，类型化枚举），返回待恢复帧
//                     （阶段 A 恒为入参帧；阶段 C 上下文切换后返回下一任务帧）
//   arm_timer()     — SBI set_timer 武装 S-timer 中断（阶段 A 自检驱动）
//
// 内核态陷阱约定：现场保存在内核帧（TRAP_CONTEXT VA），处理器运行在 per-hart
// trap 栈上；入口硬件已清 SIE，处理器内嵌套陷阱仅可能是内核 bug，会覆写内核
// 帧（panic 兜底）。trap 栈底 canary 在处理器出入口校验（溢出即 panic）。

use core::sync::atomic::{AtomicUsize, Ordering};

use log::info;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sie, stval, stvec, time};

use crate::ecall::{TimerCall, fid::Timer, scall::SArgs};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::space::{TRAP_CONTEXT, kernel_trap_context};
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::{
    __trampoline_end, __trampoline_start, alltraps_va, trap_stack_bottom, trap_stack_top,
};

/// per-hart trap 栈底 canary（溢出检测：破坏即 panic）。
const TRAP_STACK_CANARY: usize = 0x5EED_CAFE_51A7_0000;

/// 阶段 A 自检定时器周期（QEMU virt 时钟约 10 MHz → 0.1 s）。
pub const TIMER_INTERVAL: usize = 1_000_000;

/// 定时器 tick 计数（自检：前若干次打印后静默）。
static TICKS: AtomicUsize = AtomicUsize::new(0);

/// trap 栈已用字节数（高水位跟踪用）。
fn trap_stack_used() -> usize {
    let sp: usize;
    // SAFETY: 读当前栈指针，纯读无副作用。
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    trap_stack_top().saturating_sub(sp)
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
                    putln!("trap stack high-water: {used} B / 16 KiB");
                }
                break;
            }
            Err(current) => peak = current,
        }
    }
}

/// 初始化 trap 运行时（须在 `manager::init` 之后：内核帧与 TRAMPOLINE 映射已就绪）。
pub fn init() {
    // 0. 防呆：trampoline 汇编必须落在一页内（TRAMPOLINE 映射只覆盖一页）
    let tsize = (core::ptr::addr_of!(__trampoline_end) as usize)
        - (core::ptr::addr_of!(__trampoline_start) as usize);
    assert!(
        tsize <= PAGE_SIZE,
        "trampoline exceeds one page: {tsize:#x}"
    );

    // 1. 内核 trap-context 帧元数据（帧由 manager::init 映射，PA 已发布）
    let ktc = kernel_trap_context();
    let frame = unsafe { &mut *(ktc.as_usize() as *mut TrapContext) };
    let ksatp = satp::read().bits();
    frame.kernel_satp = ksatp;
    frame.kernel_sp = VirtAddr::from_raw(trap_stack_top());
    frame.trap_handler = VirtAddr::from_raw(trap_handler as *const () as usize);
    frame.trap_stack_corrupt = TRAP_STACK_CANARY;
    frame.user_pa = ktc;
    frame.user_satp = ksatp;
    // self_va：内核帧恒在 TRAP_CONTEXT VA（restore 切表后经此收尾）
    frame.self_va = TRAP_CONTEXT.as_usize();

    // 2. per-hart trap 栈底 canary（内核恒等映射，链接地址即物理地址）
    unsafe {
        (trap_stack_bottom() as *mut usize).write(TRAP_STACK_CANARY);
    }

    // 3. 先武装定时器：OpenSBI 可能遗留一个已到期的 stimecmp，若不清掉，
    //    开中断瞬间会立即触发一次 S-timer 陷阱（无害但时序难看）。
    arm_timer(TIMER_INTERVAL);

    // 4. stvec → __alltraps（Direct 模式）；sscratch = 0（内核态约定）；
    //    使能定时器源：sie.STIE。**不**开 sstatus.SIE（全局）——内核态恒关中断
    //    （处理器内关中断策略）：SIE 只经 sret 由帧内 SPIE 恢复，用户态 = 1，
    //    内核态 = 0。故 S-timer 只在用户态触发，内核代码永不被打断/抢占。
    unsafe {
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        core::arch::asm!("csrw sscratch, zero");
        sie::set_stimer();
    }

    info!(
        "runtime: trap vector {:#x}, kernel frame {:#x}, trap stack {:#x}..{:#x}",
        alltraps_va(),
        ktc.as_usize(),
        trap_stack_bottom(),
        trap_stack_top()
    );
}

/// 武装 S-timer 中断：`stimecmp = time + interval`（SBI TIME 扩展，绝对时间）。
pub fn arm_timer(interval: usize) {
    let next = time::read() + interval;
    TimerCall::new(Timer::SetTimer)
        .args(SArgs {
            a0: next,
            ..Default::default()
        })
        .call()
        .unwrap();
}

/// 定时器 tick 计数（ENV_GET_TICKS 后端）。
pub fn ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

/// 陷阱分发 — 汇编入口（`jalr trap_handler`）的唯一 Rust 侧。
///
/// 入参 `frame` = 被中断上下文的帧（汇编以 a0 = 帧物理地址调用，恒等映射下
/// 引用即物理地址）；返回值 = 待恢复帧（阶段 A 恒为入参帧；阶段 C 可返回
/// 下一任务帧实现切换）。
///
/// # Safety
///
/// 仅由 trampoline 汇编调用：入参必须指向有效且独占的 `TrapContext`（帧独占性
/// 由汇编入口/出口顺序保证——每次陷阱新建引用，无并发别名），且当前处于陷阱
/// 上下文（中断屏蔽、CSR 已由硬件保存）。
#[unsafe(no_mangle)]
extern "C" fn trap_handler(frame: &mut TrapContext) -> *mut TrapContext {
    // 入口校验：trap 栈 canary 与内核帧标记（上一次处理器若溢出，此处立即暴露）
    let canary = unsafe { (trap_stack_bottom() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted (overflow?)"
    );
    assert_eq!(
        frame.trap_stack_corrupt, TRAP_STACK_CANARY,
        "kernel trap frame corrupted"
    );
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
            let tick = TICKS.fetch_add(1, Ordering::Relaxed);
            if tick < 8 {
                putln!("timer tick #{tick} @ time={}", time::read());
            }
            arm_timer(TIMER_INTERVAL);
            if frame.sstatus & (1 << 8) == 0 {
                crate::task::tick() as *mut TrapContext
            } else {
                frame as *mut TrapContext
            }
        }
        Trap::Interrupt(other) => {
            putln!("unhandled interrupt: {other:?}");
            frame as *mut TrapContext
        }
        // 用户态环境调用（U 态 ecall）：envcall 表分发
        Trap::Exception(Exception::UserEnvCall) => crate::task::envcall::dispatch(frame),
        // 用户态缺页：机制归 memory::fault，策略归 trap 层（解析失败即 panic）
        Trap::Exception(
            Exception::InstructionPageFault | Exception::LoadPageFault | Exception::StorePageFault,
        ) => {
            let fault = unsafe { crate::memory::manager::fault::PageFault::capture() };
            let ok = crate::task::with_current_space(|space| {
                crate::memory::manager::fault::handle_page_fault(&fault, space)
            });
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
    let canary = unsafe { (trap_stack_bottom() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted after handler"
    );

    next
}
