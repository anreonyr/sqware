//! 本核的空闲环：先在自己槽里找，找不到就偷、就睡，醒来再看。

use riscv::register::{sie, sip};

use crate::core::work::room::deadline;
use crate::core::work::room::halt::{self, beacon};
use crate::core::work::room::messenger;
use crate::core::work::room::scheduler;
use crate::core::work::unit::tally;
use crate::machine::hart;

use super::dispatch::dispatch;

/// 没在停机路上时可以一觉睡到天亮的时限。
const WFI_FAR: u64 = 1 << 60;
/// 停机路上改按这个时限回来看一眼。
const BEACON_TICK: u64 = 250_000;

pub(super) fn fetch() -> usize {
    #[cfg(debug_assertions)]
    crate::core::work::assert_unlocked();
    loop {
        unsafe {
            sie::set_sext();
        }
        if let Some(pa) = dispatch() {
            return pa;
        }
        if tally::done() {
            halt::halt();
        }
        if let Some(pa) = wait() {
            return pa;
        }
    }
}

fn wait() -> Option<usize> {
    let me = hart::hart_id();
    scheduler::idle::sleep(me);
    let found = dispatch();
    if let Some(pa) = found {
        scheduler::idle::wake(me);
        return Some(pa);
    }
    if tally::done() {
        halt::halt();
    }
    loop {
        if tally::done() {
            unsafe { sip::clear_ssoft() };
            scheduler::idle::wake(me);
            halt::halt();
        }
        beacon::idle(me);

        let fallback = if beacon::shutting_down() {
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
        if let Some(pa) = dispatch() {
            scheduler::idle::wake(me);
            return Some(pa);
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
        if let Some(pa) = dispatch() {
            scheduler::idle::wake(me);
            return Some(pa);
        }
        unsafe { sip::clear_ssoft() };
    }
    unsafe { sip::clear_ssoft() };
    scheduler::idle::wake(me);
    None
}
