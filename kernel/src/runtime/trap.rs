// 陷阱处理 — stvec 接线、内核帧元数据、trap 分发（trap 运行时的 Rust 侧）
//
// 职责：
//   init()          — 写内核 trap-context 帧元数据、per-hart trap 栈 canary、
//                     stvec → __alltraps、sscratch 清 0、开 SIE
//   trap_handler()  — 汇编入口的 Rust 分发（scause 解码），返回待恢复帧
//                     （阶段 A 恒为入参帧；阶段 C 上下文切换后返回下一任务帧）
//   arm_timer()     — SBI set_timer 武装 S-timer 中断（阶段 A 自检驱动）
//
// 内核态陷阱约定：现场保存在内核帧（TRAP_CONTEXT VA），处理器运行在 per-hart
// trap 栈上；入口硬件已清 SIE，处理器内嵌套陷阱仅可能是内核 bug，会覆写内核
// 帧（panic 兜底）。trap 栈底 canary 在处理器出入口校验（溢出即 panic）。

use core::sync::atomic::{AtomicUsize, Ordering};

use log::info;
use riscv::register::{
    satp,
    scause::{self, Trap},
    sepc, sie, sstatus, stval, stvec, time,
};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::space::kernel_trap_context;
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::{
    __trampoline_end, __trampoline_start, alltraps_va, trap_stack_bottom, trap_stack_top,
};
use crate::sbi::{TimerCall, fid::Timer, scall::SArgs};

/// per-hart trap 栈底 canary（溢出检测：破坏即 panic）。
const TRAP_STACK_CANARY: usize = 0x5EED_CAFE_51A7_0000;

/// 阶段 A 自检定时器周期（QEMU virt 时钟约 10 MHz → 0.1 s）。
pub const TIMER_INTERVAL: usize = 1_000_000;

/// 定时器 tick 计数（自检：前若干次打印后静默）。
static TICKS: AtomicUsize = AtomicUsize::new(0);

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
    frame.self_pa = ktc;
    frame.user_satp = ksatp;

    // 2. per-hart trap 栈底 canary（内核恒等映射，链接地址即物理地址）
    unsafe {
        (trap_stack_bottom() as *mut usize).write(TRAP_STACK_CANARY);
    }

    // 3. 先武装定时器：OpenSBI 可能遗留一个已到期的 stimecmp，若不清掉，
    //    开中断瞬间会立即触发一次 S-timer 陷阱（无害但时序难看）。
    arm_timer(TIMER_INTERVAL);

    // 4. stvec → __alltraps（Direct 模式）；sscratch = 0（内核态约定）；
    //    开中断：sie.STIE（S-timer 源）+ sstatus.SIE（全局）
    unsafe {
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        core::arch::asm!("csrw sscratch, zero");
        sie::set_stimer();
        sstatus::set_sie();
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

/// 陷阱分发 — 汇编入口（`jalr trap_handler`）的唯一 Rust 侧。
///
/// 入参 `frame_pa` = 被中断上下文的帧物理地址（self_pa，内核恒等映射可访问）；
/// 返回值 = 待恢复帧（阶段 A 恒为入参帧；阶段 C 可返回下一任务帧实现切换）。
///
/// # Safety
///
/// 仅由 trampoline 汇编调用：入参必须是合法帧地址，且当前处于陷阱上下文
/// （中断屏蔽、CSR 已由硬件保存）。
#[unsafe(no_mangle)]
extern "C" fn trap_handler(frame_pa: usize) -> usize {
    let frame = unsafe { &mut *(frame_pa as *mut TrapContext) };
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

    // scause 裸码值：5 = SupervisorTimer 中断；8 = ecall；12/13/15 = 缺页
    match scause::read().cause() {
        Trap::Interrupt(code) => match code {
            5 => {
                let tick = TICKS.fetch_add(1, Ordering::Relaxed);
                if tick < 8 {
                    putln!("timer tick #{tick} @ time={}", time::read());
                }
                arm_timer(TIMER_INTERVAL);
            }
            other => {
                putln!("unhandled interrupt: code={other}");
            }
        },
        Trap::Exception(code) => {
            putln!(
                "kernel exception: code={code} at sepc={:#x}, stval={:#x}",
                sepc::read(),
                stval::read()
            );
            panic!("unhandled kernel exception");
        }
    }

    // 出口再校验一次 canary（处理器自身栈用量引发的溢出）
    let canary = unsafe { (trap_stack_bottom() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted after handler"
    );

    frame_pa
}
