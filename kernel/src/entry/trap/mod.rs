pub(crate) mod fault;
pub(crate) mod fault_wait;

use alloc::sync::Arc;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{scause, sepc, sie, sip, stval};

use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::identity::{self as asid, Asid};
use crate::core::memory::nest::{FaultOutcome, Nest};
use crate::core::work::record::trace::{self, EventKind, MemoryEvent, RoomEvent};
use crate::core::work::room::conductor::run::run;
use crate::core::work::room::deadline;
use crate::core::work::room::identity::{Identity, ident};
use crate::core::work::room::messenger::{self, Handoff, WakeKey, redeem};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::hart::{self, HartId};
use crate::machine::switcher::context::TrapContext;
use crate::putln;

pub(crate) mod resources;
#[cfg(debug_assertions)]
use crate::machine::switcher::stack::trap_stack_edge;
use crate::machine::switcher::stack::{
    TRAP_STACK_CANARY, trap_stack_base, trap_stack_guard_hart, trap_stack_hart,
};

pub(crate) fn persist(frame: &TrapContext) -> bool {
    let Some(i) = ident() else {
        return false;
    };
    let Some(task) = i.live() else {
        return false;
    };
    if !task.team.space.nest.asid().is_kernel() {
        return false;
    }
    let Some(pa) = i.trap() else {
        return false;
    };
    let dst = pa.as_usize() as *mut TrapContext;
    unsafe {
        (*dst).gpr = frame.gpr;
        (*dst).sstatus = frame.sstatus;
        (*dst).sepc = frame.sepc;
    }
    true
}

fn hart_frame_pa() -> usize {
    unsafe {
        (*(hart::hart_frame().as_usize() as *const TrapContext))
            .user_pa
            .as_usize()
    }
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn trap_handler(frame: &mut TrapContext) -> *mut TrapContext {
    let sp: usize;
    unsafe {
        ::core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    let hart = trap_stack_hart(sp).unwrap_or(HartId::new(0));
    let tp = crate::machine::hart::per_hart_ptr(hart);
    unsafe {
        ::core::arch::asm!("mv tp, {}", in(reg) tp, options(nomem, nostack, preserves_flags));
    }

    let frame_va = crate::machine::hart::hart_frame().as_usize();
    unsafe {
        ::core::arch::asm!(
            "csrw sscratch, {}",
            in(reg) frame_va,
            options(nomem, nostack, preserves_flags)
        );
    }

    asid::occupy(Asid::kernel());

    let from_task = (frame as *const TrapContext as usize) != hart_frame_pa();

    let ident = ident();

    crate::machine::stop::hush();

    let cause = scause::read();
    if cause.is_exception() && matches!(cause.code(), 12 | 13 | 15) {
        let stv = stval::read();
        if let Some(h) = trap_stack_guard_hart(stv) {
            panic!("trap stack overflow on hart {h} (stval = {stv:#x})");
        }
    }

    let me = hart::hart_id();
    let canary = unsafe { (trap_stack_base(me).as_usize() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} (overflow?)"
    );
    assert_eq!(
        frame.trap_stack_corrupt, TRAP_STACK_CANARY,
        "kernel trap frame corrupted"
    );
    #[cfg(debug_assertions)]
    if let Some(i) = ident.as_ref()
        && from_task
    {
        let sp: usize;
        unsafe {
            ::core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
        }
        let top = trap_stack_edge(me).as_usize();
        let ksp = frame.kernel_sp.as_usize();
        debug_assert!(
            sp <= top && top - sp < 0x4000,
            "user trap on hart {me}: sp={sp:#x} top={top:#x} frame.kernel_sp={ksp:#x} (task #{}) — kernel_sp per-switch write missing?",
            i.task_id()
        );
    }

    let trap: Trap<Interrupt, Exception> = scause::read().cause().try_into().unwrap_or_else(|e| {
        panic!("unknown trap cause: {e:?}");
    });
    let next: *mut TrapContext = match trap {
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            deadline::tick();
            unsafe {
                sie::set_sext();
            }
            deadline::beat_until(deadline::blind_ceiling());
            redeem();
            if let Some(running) = ident.as_ref().and_then(Identity::live)
                && let Some(reason) = crate::core::work::room::messenger::take_doomed(running.id)
            {
                return quit(ident, reason);
            }
            if from_task {
                run() as *mut TrapContext
            } else if persist(frame) {
                run() as *mut TrapContext
            } else {
                frame as *mut TrapContext
            }
        }
        Trap::Interrupt(Interrupt::SupervisorSoft) => {
            unsafe {
                sip::clear_ssoft();
            }
            if let Some(running) = ident.as_ref().and_then(Identity::live)
                && let Some(reason) = crate::core::work::room::messenger::take_doomed(running.id)
            {
                return quit(ident, reason);
            }
            if from_task {
                run() as *mut TrapContext
            } else {
                frame as *mut TrapContext
            }
        }
        Trap::Interrupt(Interrupt::SupervisorExternal) => {
            let result = crate::core::work::mail::nole::ring(
                &crate::core::work::room::interrupt::get().supervisor_external,
            );
            crate::core::work::room::interrupt::note_external(result.is_err(), false);
            if result.is_err() {
                unsafe {
                    sie::clear_sext();
                }
            }
            frame as *mut TrapContext
        }
        Trap::Exception(
            Exception::Breakpoint | Exception::UserEnvCall | Exception::SupervisorEnvCall,
        ) => {
            if !from_task {
                panic!("kernel ebreak from the kernel itself");
            }
            match envcall(frame, ident) {
                Some(next) => next,
                None => crate::core::work::room::messenger::quit() as *mut TrapContext,
            }
        }
        Trap::Exception(
            Exception::InstructionPageFault | Exception::LoadPageFault | Exception::StorePageFault,
        ) => {
            if !from_task {
                panic!(
                    "kernel page fault on hart {} at sepc={:#x}, stval={:#x}",
                    hart::hart_id(),
                    sepc::read(),
                    stval::read()
                );
            }
            let fault = unsafe { fault::PageFault::capture() };
            // 先取定这一次错误的身份：真正终止任务前会先把 id 标量留下来。
            let (tid, nest) = {
                let running = ident
                    .as_ref()
                    .and_then(Identity::live)
                    .expect("user page fault without running task");
                (running.id.get(), running.team.space.nest.clone())
            };
            // 先纯探一次：不动页表就能判明该重试、该物化、该等还是不该续。
            let mut outcome = nest.probe_fault(fault.addr, fault.kind);
            if let FaultOutcome::Materialize = outcome {
                // 完整解析在提交边界之外：结失效、物化、发布，再重探一次。
                match nest.resolve_fault(fault.addr, fault.kind) {
                    Ok(next) => outcome = next,
                    // 真实资源失败：这是整份地址空间的失败，不是这一页自己失效；
                    // 照实记录，再放掉地址空间按原样终止。
                    Err(resource) => {
                        log::error!(
                            "page fault resource failure: {resource:?} at {:?} pc={:#x}",
                            fault.addr,
                            fault.pc
                        );
                        page_fault_note(&fault, false);
                        drop(nest);
                        return page_fault_killed(ident, tid, fault);
                    }
                }
            }
            match outcome {
                FaultOutcome::Resolved => {
                    page_fault_note(&fault, true);
                    resume(frame as *mut TrapContext)
                }
                // 一次完整解析之后仍要物化：并发只要求重探，与重试原指令同路，
                // 绝不与真实资源失败混同。
                FaultOutcome::Materialize => {
                    page_fault_note(&fault, false);
                    resume(frame as *mut TrapContext)
                }
                // 承载不在（墓碑或在途）：按这段关系整段等一次，醒后重试原指令。
                FaultOutcome::Wait(view) => page_fault_wait(ident, fault, nest, view, frame),
                FaultOutcome::Invalid => {
                    page_fault_note(&fault, false);
                    drop(nest);
                    page_fault_killed(ident, tid, fault)
                }
            }
        }
        Trap::Exception(other) => {
            if from_task {
                let running = ident
                    .as_ref()
                    .and_then(Identity::live)
                    .expect("user exception without running task");
                let tid = running.id.get();
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
                return kill_fault(ident);
            }
            panic!(
                "unhandled kernel exception: {other:?} at sepc={:#x}, stval={:#x}",
                sepc::read(),
                stval::read()
            );
        }
    };

    let me = hart::hart_id();
    let canary = unsafe { (trap_stack_base(me).as_usize() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} after handler"
    );

    resume(next)
}

fn resume(next: *mut TrapContext) -> *mut TrapContext {
    asid::occupy(Asid::from_raw(unsafe { (*next).user_satp.asid() }));
    next
}

/// 一次页错误的记录：`resolved` 只表示“这一次访存已经能继续”；
/// 未完的等待与重探都是 `false`，真杀另由 `FaultKilled` 说明。
fn page_fault_note(fault: &fault::PageFault, resolved: bool) {
    trace::note(EventKind::Memory(MemoryEvent::PageFault {
        va: fault.addr.as_usize(),
        fault: fault.kind,
        resolved,
    }));
}

/// 按这段关系整段等一次承载；登记与切换都不带着调用者的强引用。
fn page_fault_wait(
    ident: Option<Identity>,
    fault: fault::PageFault,
    nest: Arc<Nest>,
    view: usize,
    frame: &mut TrapContext,
) -> *mut TrapContext {
    // 登记失败要终止任务时，身份已经交出去了：先把这一 id 标量留下来。
    let tid = ident.as_ref().map_or(0, Identity::task_id);
    let (space, life) = {
        let running = ident
            .as_ref()
            .and_then(Identity::live)
            .expect("user page fault without running task");
        (running.team.space.clone(), running.team.space.life())
    };
    let key = WakeKey::View {
        space: nest.asid(),
        view,
    };
    let waiting = fault_wait::FaultWait::new(nest, fault.addr, fault.kind, view);
    page_fault_note(&fault, false);
    // 先放掉空间与身份，再进提交边界的 wait_until；等待捕获自己在其内部释放。
    drop(space);
    drop(ident);
    match messenger::wait_until::<env::RoomFail, _>(
        key,
        life,
        waiting,
        fault_wait::FaultWait::ready,
    ) {
        Ok(Handoff::Resume(())) => resume(frame as *mut TrapContext),
        Ok(Handoff::Switch(pa)) => pa as *mut TrapContext,
        Err(resource) => {
            log::error!("page fault wait could not be registered: {resource:?}");
            page_fault_killed(None, tid, fault)
        }
    }
}

/// 按原样处置：记录真杀事件并终止这个任务。
fn page_fault_killed(
    ident: Option<Identity>,
    tid: usize,
    fault: fault::PageFault,
) -> *mut TrapContext {
    let cause_bits = scause::read().bits();
    let stval_bits = stval::read();
    log::error!(
        "user page fault killed: {:?} at {:?} pc={:#x}",
        fault.kind,
        fault.addr,
        fault.pc
    );
    trace::note(EventKind::Room(RoomEvent::FaultKilled {
        tid,
        cause: cause_bits,
        stval: stval_bits,
    }));
    putln!("user fault killed: tid={tid} cause={cause_bits} stval={stval_bits:#x}");
    kill_fault(ident)
}

fn quit(ident: Option<Identity>, reason: usize) -> *mut TrapContext {
    messenger::set_exit_reason(reason);
    drop(ident);
    resume(messenger::quit() as *mut TrapContext)
}

fn kill_fault(ident: Option<Identity>) -> *mut TrapContext {
    let live = match &ident {
        Some(Identity::Live(live)) => Some(live),
        _ => None,
    };
    doom(live, env::ExitCause::Fault, messenger::EXIT_FAULT);
    quit(ident, messenger::EXIT_FAULT)
}

/// 环境调用那一支：先结清这张表上待结算的失效、再按真实长度把 `sepc` 推过那条
/// `ebreak`，最后才派发。
///
/// 三件“不知道”都不许往下走——失效结不掉、入口那个字节读不出来、推过之后
/// 越出地址宽度。那是这份地址空间的真实失败，与缺页那条同一种归责：照实记录，
/// 把这个任务**显式判死**（`Fault`），交回 `None` 让入口按原样把它换出去。
/// 绝不猜一个长度推进 `sepc`（错位会让客人跳过它自己写回答案的那一存），
/// 也绝不在没结清的表上做这一次调用；越界时连原 `sepc` 一并留在原地。
pub(crate) fn envcall(
    frame: &mut TrapContext,
    ident: Option<Identity>,
) -> Option<*mut TrapContext> {
    let Some(Identity::Live(ident)) = ident else {
        panic!("envcall without running task");
    };
    let space = &ident.team.space;
    if let Err(resource) = space.nest.retry_invalidation() {
        log::error!("envcall: the address space cannot settle its sync: {resource:?}");
        doom(Some(&ident), env::ExitCause::Fault, messenger::EXIT_FAULT);
        return None;
    }
    let Some(length) = instr_len(space, frame.sepc) else {
        log::error!(
            "envcall: the instruction at sepc={:#x} is unreadable",
            frame.sepc.as_usize()
        );
        doom(Some(&ident), env::ExitCause::Fault, messenger::EXIT_FAULT);
        return None;
    };
    let Some(next_pc) = frame.sepc.as_usize().checked_add(length) else {
        log::error!(
            "envcall: sepc={:#x} plus an instruction length overflows",
            frame.sepc.as_usize()
        );
        doom(Some(&ident), env::ExitCause::Fault, messenger::EXIT_FAULT);
        return None;
    };
    frame.sepc = KVirt::wrap(next_pc);
    crate::entry::call::dispatch(frame, ident)
}

/// 环境调用那条指令的长度：`c.ebreak` 两字节、`ebreak` 四字节。
///
/// 首字节读不出来就是**不知道**，交 `None`，不凭空造一个长度。客人的正文只要
/// 在跑就读得出来（这条 `ebreak` 刚执行过）；读不出来是这份地址空间的失败。
fn instr_len(space: &Space, sepc: KVirt) -> Option<usize> {
    let byte = space.instruction_byte(sepc)?;
    Some(if byte & 0b11 == 0b11 { 4 } else { 2 })
}

/// 把这个身份指到的任务显式判死并留下原因（换出由调用处负责）。
///
/// 身份指得到就只落在那一个上（环境调用与缺页两条路都按这份身份归责）；
/// 指不到时才落到此刻在跑的那一个。
fn doom(ident: Option<&Arc<TaskIdent>>, cause: env::ExitCause, reason: usize) {
    let task = match ident {
        Some(ident) => crate::core::work::room::scheduler::roster::muster(ident.id)
            .and_then(|weak| weak.upgrade()),
        None => None,
    }
    .or_else(|| crate::core::work::room::conductor::current().running_task());
    let Some(task) = task else { return };
    let _commit = crate::core::work::commit();
    if !task.dead() {
        task.state
            .lock()
            .doom(Some(crate::machine::hart::hart_id()), cause, reason);
    }
}
