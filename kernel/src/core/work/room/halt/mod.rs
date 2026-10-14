//! 停机：没有活可跑了 → 各核各自到达 → 等齐 → 最后一核收尾 → 复位。
//!
//! 收尾那几件（报告、拆台、flush、判定）由 `entry` 在 boot 时装进来，本模块不认识
//! 它们；`beacon` 是停机路上的信标：根没了、久无进展，就报一次。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sbi::{self, fid};

use crate::machine::hart;
use crate::machine::sync::OnceLock;

use super::scheduler;

pub(crate) mod beacon;

/// 等齐期间报一次的时机（自旋次数）。
const BARRIER_REPORT_AT: usize = 20_000_000;

static HALTING: AtomicBool = AtomicBool::new(false);
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);

type Hook = fn();

static HOOKS: OnceLock<(&'static [Hook], fn())> = OnceLock::new();

/// 装停机收尾：`hooks` 等齐后按序跑，`waiting` 在屏障久等时报一次。
pub(crate) fn hook(hooks: &'static [Hook], waiting: fn()) {
    let _ = HOOKS.set((hooks, waiting));
}

/// 屏障读数：已到达的核数 / 应有的核数。
pub(crate) fn barrier() -> (usize, usize) {
    (
        HALT_ARRIVED.load(Ordering::Acquire),
        hart::hart_count(),
    )
}

/// 本核到达停机屏障，不再返回：交还 ASID，等其余核到齐，由最后一核收尾并复位。
pub(super) fn halt() -> ! {
    crate::core::memory::manager::asid::vacate();
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        scheduler::idle::yell();
        let mut spins = 0usize;
        let mut reported = false;
        while HALT_ARRIVED.load(Ordering::Acquire) < hart::hart_count() {
            spins += 1;
            if !reported && spins == BARRIER_REPORT_AT {
                reported = true;
                if let Some((_, waiting)) = HOOKS.get() {
                    waiting();
                }
            }
            core::hint::spin_loop();
        }
        hooked();
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    }
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

fn hooked() {
    if let Some((hooks, _)) = HOOKS.get() {
        for h in hooks.iter() {
            h();
        }
    }
}
