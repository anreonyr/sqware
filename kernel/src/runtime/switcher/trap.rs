//! trap 分发 — 汇编入口（`jalr trap_handler`）的唯一 Rust 侧，兼内核态现场持久化。
//!
//! 陷阱栈的窗口与反解在 `stack` 子模块；本文件只用它的符号，公开路径经此处转出。

use core::time::Duration;

use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{scause, sepc, sip, stval};

use crate::memory::manager::asid::{self, Asid};
use crate::putln;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, MemoryEvent, RoomEvent};
use crate::runtime::switcher::context::TrapContext;
use crate::work::room::messenger::redeem;
use crate::work::room::scheduler::core::{Identity, ident};
use crate::work::room::scheduler::trap::run;
use crate::{machine, put};

mod stack;

// trap 栈的公开符号经此处转出 —— 对外的 `switcher::trap::trap_stack_*` 路径不变。
pub(crate) use stack::{TRAP_STACK_CANARY, trap_stack_guard_hart, trap_stack_hart};
pub use stack::{arm_hart, init, trap_stack, trap_stack_base, trap_stack_edge};

/// 内核态被打断的现场持久化：把 hart 帧（仅一份）的被中断现场
/// （gpr/sstatus/sepc）拷入当前 running 任务的专属帧。否则抢占切走后再来
/// 陷阱会覆写 hart 帧——被抢占内核任务的现场将丢失。
///
/// 判定源（与调度域的 D2-1 收敛一致）：「running 任务是内核任务」由任务所属
/// 空间的 kind 决定（不再读 sstatus.spp）。硬件抢占与自愿切换（S 态域任务发
/// envcall 切走）共用本搬移。
///
/// 自查询形态：身份经 `ident()` 无锁读槽（软陷阱那套汇编入口已随内核任务面删除，
/// 现在唯一调用点也传不了参——槽读廉价、崩溃现场安全）。只搬三个现场字段；
/// 任务帧其余元数据（kernel_satp/kernel_sp/trap_handler/user_satp/self_va/…）由
/// spawn/prepare 维护，不得改动。
pub(crate) fn persist(frame: &TrapContext) -> bool {
    let Some(i) = ident() else {
        return false;
    };
    // Live 轴才有地址空间与 trap 帧：S 态空闲（末次身份）时不得写入——帧可能
    // 已被 clear 归还重分配，写即覆写他人帧（评审漏掉的第二个悬垂点，写侧）。
    let Some(task) = i.live() else {
        return false;
    };
    if !task.team.space.asid().is_kernel() {
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

    // 0.4 入场入册：`__task_trap`/`__core_trap` 已整表刷（不变量 1），本核转为
    //     内核租户（内核空间身份 ASID 0）。
    asid::set_asid(Asid::kernel());

    // 0.45 陷阱来源：`__core_trap` 传本 hart 帧、`__task_trap` 传任务帧。这是
    //     「被中断者是内核还是任务」的**唯一判据**——S 态 supervisor 域任务的
    //     SPP 也是 Supervisor，不能靠 SPP 区分（域任务必须能抢占、能缺页自愈、
    //     能 ecall）。
    let from_task = (frame as *const TrapContext as usize) != machine::hart_frame().as_usize();

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
    // 2. debug：任务（U 态 / S 态域任务）陷阱必须运行在当前 hart 的 trap 栈上
    //    （kernel_sp 每次切换写入的正确性——任务迁移后写漏即在此暴露）。
    #[cfg(debug_assertions)]
    if let Some(i) = ident.as_ref()
        && from_task
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
            timer::beat(clock::duration_to_ticks(Duration::from_millis(100)));
            redeem();
            if from_task {
                // 任务（U 态或 S 态域任务）被抢占：现场已在任务帧 → 直接切换
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
            // 风暴）。若本核当前 running 任务被 `doomed` 点名（kill 的他核分支），
            // 在此自退：quit + bury，再取下一任务。
            unsafe {
                sip::clear_ssoft();
            }
            if let Some(running) = ident.as_ref().and_then(Identity::live)
                && crate::work::room::messenger::take_doomed(running.id)
            {
                drop(ident);
                return crate::work::room::messenger::quit() as *mut TrapContext;
            }
            frame as *mut TrapContext
        }
        Trap::Interrupt(other) => {
            put!("unhandled interrupt: {other:?}\n{frame:#?}\n");
            frame as *mut TrapContext
        }
        // 任务环境调用（`ebreak`，scause=3）：U 态任务与 S 态 supervisor 域任务
        // 共用同一入口（`ecall` 不行——S 态 ecall 是 SBI 调用，进 M 态）。内核
        // 自身 ebreak 不应出现（semihosting 由 QEMU 拦截，不经本路径）→ 内核 bug。
        // 身份 Arc **移交**给 dispatch：其内部在可能触发 halt（run）的分支（Reap/
        // Park/Wait）先 drop——否则 halt 时本核 trap_handler 仍持最后任务的
        // Arc<TaskIdent> → team → space 被钉住不 drop，关机审计误报帧泄漏。
        Trap::Exception(
            Exception::Breakpoint | Exception::UserEnvCall | Exception::SupervisorEnvCall,
        ) => {
            if !from_task {
                panic!("kernel ebreak from the kernel itself");
            }
            let Some(Identity::Live(ident_arc)) = ident else {
                panic!("envcall without running task");
            };
            match crate::runtime::switcher::envcall::dispatch(frame, ident_arc) {
                Some(next) => next,
                // **退场窄尾**：dispatch 的帧此刻已归还（它的局部量照常 drop），
                // 退场发生在本帧——本帧手里只有 frame 与几个标量，`ident` 已移交。
                None => crate::work::room::messenger::quit() as *mut TrapContext,
            }
        }
        // 任务缺页：解析成功 → 续跑；解析失败 → fault isolation 杀 task。
        // 内核自身缺页 = 内核 bug → 仍 panic。
        Trap::Exception(
            Exception::InstructionPageFault | Exception::LoadPageFault | Exception::StorePageFault,
        ) => {
            if !from_task {
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
                .and_then(Identity::live)
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
            putln!("user fault killed: tid={tid} cause={cause_bits} stval={stval_bits:#x}");
            drop(ident);
            return crate::work::room::messenger::quit() as *mut TrapContext;
        }
        // 异常：任务（U 态 / S 态域任务）→ fault isolation 杀 task；内核自身 → fatal。
        Trap::Exception(other) => {
            if from_task {
                let running = ident
                    .as_ref()
                    .and_then(Identity::live)
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
                return crate::work::room::messenger::quit() as *mut TrapContext;
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

    // 出场登记：本核将驻留**下一帧**的空间（`run()` 可能已换任务），且必须在
    // 返回之前——`__restore` 的 sfence 后本核就带新 ASID 的 TLB，RFENCE 清退
    // 需能在该时刻正确发现本核驻留该 ASID。
    // SAFETY: next 恒指向本核有效帧（分发各分支的产物），恒等映射下可解引用。
    asid::set_asid(Asid::from_raw(unsafe { (*next).user_satp.asid() }));

    next
}
