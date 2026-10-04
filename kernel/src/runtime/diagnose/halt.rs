use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::hart;
use crate::layout::ROOT_STACK_SIZE;
use crate::runtime::diagnose::report::Report;
use sbi::{self, ecall::SArgs, fid};

static ALARM: AtomicBool = AtomicBool::new(false);
static ALARMER: AtomicUsize = AtomicUsize::new(usize::MAX);

pub fn hush() {
    if ALARM.load(Ordering::Acquire) && ALARMER.load(Ordering::Acquire) != hart::hart_id().get() {
        hunker();
    }
}

fn hunker() -> ! {
    crate::memory::manager::asid::vacate();
    // SAFETY: 仅清 sstatus.SIE（写本 hart CSR）
    unsafe { core::arch::asm!("csrci sstatus, 2") };
    let _ = sbi::HsmCall::new(fid::Hsm::Stop).call();
    loop {
        core::hint::spin_loop();
    }
}

fn claim() -> bool {
    if ALARM.swap(true, Ordering::AcqRel) {
        if ALARMER.load(Ordering::Acquire) != hart::hart_id().get() {
            hunker();
        }
        return false;
    }
    ALARMER.store(hart::hart_id().get(), Ordering::Release);
    true
}

fn broadcast() {
    let me = hart::hart_id();
    let n = hart::hart_count();
    for w in 0..n.div_ceil(usize::BITS as usize) {
        let base = w * (usize::BITS as usize);
        let hi = (base + (usize::BITS as usize)).min(n).saturating_sub(base);
        let mut mask = 0usize;
        for b in 0..hi {
            let hart = base + b;
            if crate::hart::HartId::new(hart) != me {
                mask |= 1usize << b;
            }
        }
        if mask == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: mask,
                a1: base,
                ..Default::default()
            })
            .call();
    }
}

fn alarm() -> bool {
    let won = claim();
    if won {
        broadcast();
    }
    won
}

static mut SCENE: [usize; 2] = [0; 2];

pub(crate) fn scene() -> (usize, usize) {
    // SAFETY: 单写单读同 hart；volatile 防 asm 写入被缓存
    let s = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCENE)) };
    (s[0], s[1])
}

#[panic_handler]
pub(crate) fn panic_handler(info: &PanicInfo) -> ! {
    if crate::testing() {
        crate::putln!("test panic: {info}");
        semihosting::process::abort()
    }
    crash_scene(info)
}

fn crash_scene(info: &PanicInfo) -> ! {
    if !alarm() {
        crate::putln!(
            "info: {} sepc={:#x} stval={:#x}",
            info.message(),
            riscv::register::sepc::read(),
            riscv::register::stval::read(),
        );
        halt_loop()
    }
    home(info)
}

#[allow(improper_ctypes_definitions)]
#[unsafe(naked)]
extern "C" fn home(_info: &PanicInfo) -> ! {
    core::arch::naked_asm!(
        "la   t0, {scene}",
        "sd   sp, 0(t0)",
        "sd   s0, 8(t0)",
        "la   t0, _kernel_edge",
        "li   t1, {size}",
        "add  t1, t0, t1",
        "mv   t2, sp",
        "bltu t2, t0, 1f",
        "bltu t2, t1, 2f",
        "1:  mv   sp, t1",
        "2:  tail {work}",
        scene = sym SCENE,
        size = const ROOT_STACK_SIZE,
        work = sym info,
    );
}

#[allow(improper_ctypes_definitions)]
extern "C" fn info(info: &PanicInfo) -> ! {
    // SAFETY: volatile 读 root 栈底哨兵
    let root_ok = unsafe { (crate::layout::root_stack_base() as *const usize).read_volatile() }
        == crate::layout::ROOT_STACK_CANARY;

    crate::memory::allocator::portal::switch(crate::memory::allocator::portal::Backend::Spare);

    let mut report = Report::default();
    {
        let mut head = String::from("[panic]");
        if let Some(loc) = info.location() {
            head.push_str(&format!(
                " at {}:{}:{}",
                loc.file(),
                loc.line(),
                loc.column()
            ));
        }
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();
        if let Some(i) = crate::work::room::scheduler::core::ident() {
            rows.push(vec![Some(format!(
                "team #{} / task #{} @ hart {}",
                i.team_id(),
                i.task_id(),
                hart::hart_id()
            ))]);
        }
        rows.push(vec![Some(format!("{}", info.message()))]);
        rows.push(vec![Some(format!(
            "root stack @ {} : {}",
            if root_ok { "ok" } else { "CORRUPTED" },
            hart::hart_id()
        ))]);
        report.paragraph("panic", Some(head)).items.extend(rows);
    }

    crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Halt(
        crate::runtime::diagnose::trace::HaltEvent::Panic,
    ));
    crate::runtime::diagnose::scene::dump(&mut report);

    let sealed = report.seal();
    crate::putln!();
    let mut sink = crate::console::Sink;
    crate::runtime::diagnose::render::render(sealed, &mut sink, 2);
    #[cfg(feature = "semihosting")]
    crate::runtime::diagnose::export::export(sealed);

    halt_loop()
}

pub(crate) fn stop_boot() {
    // SAFETY: disable interrupts on the reporting hart.
    unsafe { core::arch::asm!("csrci sstatus, 2") };
    if claim() && crate::platform::machine::dram_edge().is_some() {
        broadcast();
    }
}

pub(crate) fn halt_loop() -> ! {
    // SAFETY: 仅清 sstatus.SIE（写本 hart CSR）
    unsafe { core::arch::asm!("csrci sstatus, 2") };
    loop {
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
        unsafe { core::arch::asm!("wfi") };
    }
}
