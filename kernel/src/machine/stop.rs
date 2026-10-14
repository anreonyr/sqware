use ::core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crate::machine::hart;
use sbi::{self, ecall::SArgs, fid};

static ALARM: AtomicBool = AtomicBool::new(false);
static ALARMER: AtomicUsize = AtomicUsize::new(usize::MAX);

pub(crate) fn hush() {
    if ALARM.load(Ordering::Acquire) && ALARMER.load(Ordering::Acquire) != hart::hart_id().get() {
        hunker();
    }
}

fn hunker() -> ! {
    crate::core::memory::nest::identity::vacate();
    unsafe { ::core::arch::asm!("csrci sstatus, 2") };
    let _ = sbi::HsmCall::new(fid::Hsm::Stop).call();
    loop {
        ::core::hint::spin_loop();
    }
}

pub(crate) fn claim() -> bool {
    if ALARM.swap(true, Ordering::AcqRel) {
        if ALARMER.load(Ordering::Acquire) != hart::hart_id().get() {
            hunker();
        }
        return false;
    }
    ALARMER.store(hart::hart_id().get(), Ordering::Release);
    true
}

pub(crate) fn alarm() -> bool {
    let won = claim();
    if won {
        broadcast();
    }
    won
}

pub(crate) fn broadcast() {
    let me = hart::hart_id();
    let n = hart::hart_count();
    for w in 0..n.div_ceil(usize::BITS as usize) {
        let base = w * (usize::BITS as usize);
        let hi = (base + (usize::BITS as usize)).min(n).saturating_sub(base);
        let mut mask = 0usize;
        for b in 0..hi {
            if hart::HartId::new(base + b) != me {
                mask |= 1usize << b;
            }
        }
        if mask == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs { a0: mask, a1: base, ..Default::default() })
            .call();
    }
}
