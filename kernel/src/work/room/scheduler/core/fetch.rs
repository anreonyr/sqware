use alloc::sync::Arc;

use riscv::register::{sie, sip};

use crate::hart;
use crate::runtime::chrono::timer;
use crate::work::room::conductor;
use crate::work::room::messenger;
use crate::work::unit::task::Task;

use super::table::current;

const WFI_FAR: u64 = 1 << 60;
const BEACON_TICK: u64 = 250_000;

pub(in super::super) fn fetch() -> usize {
    let s = current();
    loop {
        if let Some(task) = s.pull() {
            return s.seat(task);
        }
        if conductor::done() {
            conductor::halt();
        }
        if let Some(task) = wait() {
            return s.seat(task);
        }
    }
}

fn wait() -> Option<Arc<Task>> {
    let me = hart::hart_id();
    conductor::sleep(me);
    let found = current().pull();
    if let Some(task) = found {
        conductor::wake(me);
        return Some(task);
    }
    if conductor::done() {
        conductor::halt();
    }
    loop {
        if conductor::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }
        crate::work::room::scheduler::core::beacon::idle(me);

        let fallback = if crate::work::room::scheduler::core::beacon::shutting_down() {
            BEACON_TICK
        } else {
            WFI_FAR
        };
        timer::beat_until(fallback);
        // SAFETY: 只置 sie.SEIE 一位，不改任何内存与栈
        unsafe {
            sie::set_sext();
        }
        if sip::read().sext() {
            let _ = crate::platform::devices::raise_irq_idle();
        }
        #[cfg(debug_assertions)]
        crate::runtime::diagnose::ipi::idle_hook();
        #[cfg(debug_assertions)]
        crate::runtime::diagnose::ipi::wfi_entry(me);
        unsafe {
            core::arch::asm!("wfi");
        }
        #[cfg(debug_assertions)]
        crate::runtime::diagnose::ipi::wfi_exit(me, sip::read().ssoft());
        crate::work::room::messenger::sweep_doomed();
        if messenger::redeem() {
            break;
        }
        if let Some(task) = current().pull() {
            conductor::wake(me);
            return Some(task);
        }
        // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位
        unsafe { sip::clear_ssoft() };
    }
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位
    unsafe { sip::clear_ssoft() };
    conductor::wake(me);
    None
}
