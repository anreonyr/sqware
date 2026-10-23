use ::core::panic::PanicInfo;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::machine::hart;
use crate::machine::image::ROOT_STACK_SIZE;
use crate::entry::report::report::Report;
use crate::machine::stop::{alarm, claim, broadcast};
use sbi::fid;

static mut SCENE: [usize; 2] = [0; 2];

pub(crate) fn scene() -> (usize, usize) {
    let s = unsafe { ::core::ptr::read_volatile(::core::ptr::addr_of!(SCENE)) };
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
    ::core::arch::naked_asm!(
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
    let root_ok = unsafe { (crate::machine::image::root_stack_base() as *const usize).read_volatile() }
        == crate::machine::image::ROOT_STACK_CANARY;

    crate::core::memory::flat::allocator::portal::switch(crate::core::memory::flat::allocator::portal::Backend::Spare);

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
        if let Some(i) = crate::core::work::room::identity::ident() {
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

    crate::core::work::record::trace::note(crate::core::work::record::trace::EventKind::Halt(
        crate::core::work::record::trace::HaltEvent::Panic,
    ));
    crate::entry::report::scene::dump(&mut report);

    let sealed = report.seal();
    crate::putln!();
    let mut sink = crate::machine::console::Sink;
    crate::entry::report::render::render(sealed, &mut sink, 2);
    #[cfg(feature = "semihosting")]
    crate::entry::report::export::export(sealed);

    halt_loop()
}

pub(crate) fn stop_boot() {
    unsafe { ::core::arch::asm!("csrci sstatus, 2") };
    if claim() && crate::machine::board::dram_edge().is_some() {
        broadcast();
    }
}

/// Fences other harts, then parks this one. `ALARM` is a hart boundary, not
/// evidence that a device stopped.
pub(crate) fn protected_stop(_reason: usize) -> ! {
    if !alarm() {
        crate::machine::stop::hush();
    }
    unsafe { ::core::arch::asm!("csrci sstatus, 2") };
    let expected = hart::hart_count().saturating_sub(1);
    while crate::machine::stop::quiesced_peers() != expected {
        ::core::hint::spin_loop();
    }
    #[cfg(feature = "critical-scene")]
    crate::putln!("[critical] peer harts quiesced={expected}");
    loop {
        unsafe { ::core::arch::asm!("wfi") };
    }
}

pub(crate) fn halt_loop() -> ! {
    unsafe { ::core::arch::asm!("csrci sstatus, 2") };
    loop {
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
        unsafe { ::core::arch::asm!("wfi") };
    }
}
