use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{scause, sepc, sie, sip, stval};

use crate::hart::{self, HartId};
use crate::memory::manager::asid::{self, Asid};
use crate::putln;
use crate::runtime::chrono::timer;
use crate::runtime::diagnose::trace::{self, EventKind, MemoryEvent, RoomEvent};
use crate::runtime::switcher::context::TrapContext;
use crate::work::room::messenger::{self, redeem};
use crate::work::room::scheduler::core::{Identity, ident};
use crate::work::room::scheduler::trap::run;

mod stack;

pub(crate) use stack::{TRAP_STACK_CANARY, trap_stack_guard_hart, trap_stack_hart};
pub use stack::{arm_hart, init, trap_stack, trap_stack_base, trap_stack_edge};

pub(crate) fn persist(frame: &TrapContext) -> bool {
    let Some(i) = ident() else {
        return false;
    };
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
    // SAFETY: 任务专属帧 PA 恒等映射可写；当前 running 任务独占
    unsafe {
        (*dst).gpr = frame.gpr;
        (*dst).sstatus = frame.sstatus;
        (*dst).sepc = frame.sepc;
    }
    true
}

fn hart_frame_pa() -> usize {
    // SAFETY: kernel satp 下本 hart 帧恒映射
    unsafe {
        (*(hart::hart_frame().as_usize() as *const TrapContext))
            .user_pa
            .as_usize()
    }
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn trap_handler(frame: &mut TrapContext) -> *mut TrapContext {
    let sp: usize;
    // SAFETY: 读当前栈指针
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    let hart = trap_stack_hart(sp).unwrap_or(HartId::new(0));
    let tp = crate::hart::per_hart_ptr(hart);
    // SAFETY: 写线程指针
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) tp, options(nomem, nostack, preserves_flags));
    }

    let frame_va = crate::hart::hart_frame().as_usize();
    // SAFETY: 写 sscratch（内核态约定 = 本 hart 帧 VA）
    unsafe {
        core::arch::asm!(
            "csrw sscratch, {}",
            in(reg) frame_va,
            options(nomem, nostack, preserves_flags)
        );
    }

    asid::occupy(Asid::kernel());

    let from_task = (frame as *const TrapContext as usize) != hart_frame_pa();

    let ident = ident();

    crate::runtime::diagnose::halt::hush();

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
            core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
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
            timer::tick();
            unsafe {
                sie::set_sext();
            }
            timer::beat_until(timer::blind_ceiling());
            redeem();
            if let Some(running) = ident.as_ref().and_then(Identity::live)
                && let Some(reason) = crate::work::room::messenger::take_doomed(running.id)
            {
                messenger::set_exit_reason(reason);
                drop(ident);
                return crate::work::room::messenger::quit() as *mut TrapContext;
            }
            crate::work::room::messenger::sweep_doomed();
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
                && let Some(reason) = crate::work::room::messenger::take_doomed(running.id)
            {
                messenger::set_exit_reason(reason);
                drop(ident);
                return crate::work::room::messenger::quit() as *mut TrapContext;
            }
            frame as *mut TrapContext
        }
        Trap::Interrupt(Interrupt::SupervisorExternal) => {
            if crate::platform::devices::raise_irq().is_err() {
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
            let Some(Identity::Live(ident_arc)) = ident else {
                panic!("envcall without running task");
            };
            match crate::runtime::switcher::envcall::dispatch(frame, ident_arc) {
                Some(next) => next,
                None => crate::work::room::messenger::quit() as *mut TrapContext,
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
                return frame as *mut TrapContext;
            }
            let tid = running.id.get();
            let cause_bits = scause::read().bits();
            let stval_bits = stval::read();
            trace::note(EventKind::Room(RoomEvent::FaultKilled {
                tid,
                cause: cause_bits,
                stval: stval_bits,
            }));
            putln!("user fault killed: tid={tid} cause={cause_bits} stval={stval_bits:#x}");
            messenger::set_exit_reason(messenger::EXIT_FAULT);
            drop(ident);
            return crate::work::room::messenger::quit() as *mut TrapContext;
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
                messenger::set_exit_reason(messenger::EXIT_FAULT);
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

    let me = hart::hart_id();
    let canary = unsafe { (trap_stack_base(me).as_usize() as *const usize).read() };
    assert_eq!(
        canary, TRAP_STACK_CANARY,
        "trap stack corrupted on hart {me} after handler"
    );

    // SAFETY: next 恒指向本核有效帧
    asid::occupy(Asid::from_raw(unsafe { (*next).user_satp.asid() }));

    next
}
