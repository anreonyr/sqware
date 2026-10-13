use alloc::sync::Arc;

use riscv::register::{sie, sip};

use crate::core::work::room::conductor;
use crate::core::work::room::deadline;
use crate::core::work::room::messenger;
use crate::core::work::unit::task::Task;
use crate::machine::hart;

use super::table::{current, steal};

const WFI_FAR: u64 = 1 << 60;
const BEACON_TICK: u64 = 250_000;

pub(in super::super) fn fetch() -> usize {
    #[cfg(debug_assertions)]
    crate::core::work::unit::assert_unlocked();
    let s = current();
    loop {
        unsafe {
            sie::set_sext();
        }
        if let Some(task) = pull() {
            if let Some(pa) = s.seat(task) {
                return pa;
            }
        }
        if conductor::done() {
            conductor::halt();
        }
        if let Some(task) = wait() {
            if let Some(pa) = s.seat(task) {
                return pa;
            }
        }
    }
}

fn pull() -> Option<Arc<Task>> {
    current().pull().or_else(steal)
}

fn wait() -> Option<Arc<Task>> {
    let me = hart::hart_id();
    conductor::sleep(me);
    let found = pull();
    if let Some(task) = found {
        conductor::wake(me);
        return Some(task);
    }
    if conductor::done() {
        conductor::halt();
    }
    loop {
        if conductor::done() {
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }
        crate::core::work::room::scheduler::core::beacon::idle(me);

        let fallback = if crate::core::work::room::scheduler::core::beacon::shutting_down() {
            BEACON_TICK
        } else {
            WFI_FAR
        };
        deadline::beat_until(fallback);
        unsafe {
            sie::set_sext();
        }
        if sip::read().sext() {
            let resources = crate::core::work::room::interrupt::get();
            let result = crate::core::work::mail::nole::ring(&resources.supervisor_external);
            crate::core::work::room::interrupt::note_external(result.is_err(), true);
        }
        if let Some(task) = pull() {
            conductor::wake(me);
            return Some(task);
        }
        #[cfg(debug_assertions)]
        crate::core::work::record::ipi::idle_hook();
        #[cfg(debug_assertions)]
        crate::core::work::record::ipi::wfi_entry(me);
        unsafe {
            core::arch::asm!("wfi");
        }
        #[cfg(debug_assertions)]
        crate::core::work::record::ipi::wfi_exit(me, sip::read().ssoft());
        if messenger::redeem() {
            break;
        }
        if let Some(task) = pull() {
            conductor::wake(me);
            return Some(task);
        }
        unsafe { sip::clear_ssoft() };
    }
    unsafe { sip::clear_ssoft() };
    conductor::wake(me);
    None
}
