use crate::machine::hart;
use ::core::sync::atomic::{AtomicUsize, Ordering};
use sbi::{self, ecall::SArgs, fid};

static ALARMER: AtomicUsize = AtomicUsize::new(usize::MAX);
static QUIET: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn hush() {
    let owner = ALARMER.load(Ordering::Acquire);
    if owner != usize::MAX && owner != hart::hart_id().get() {
        hunker();
    }
}

fn hunker() -> ! {
    crate::core::memory::nest::identity::vacate();
    unsafe { ::core::arch::asm!("csrci sstatus, 2") };
    // This path never returns: each peer acknowledges once after vacating its lease.
    QUIET.fetch_add(1, Ordering::Release);
    let _ = sbi::HsmCall::new(fid::Hsm::Stop).call();
    loop {
        ::core::hint::spin_loop();
    }
}

pub(crate) fn claim() -> bool {
    let me = hart::hart_id().get();
    match ALARMER.compare_exchange(usize::MAX, me, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => true,
        Err(owner) if owner == me => false,
        Err(_) => hunker(),
    }
}

pub(crate) fn quiesced_peers() -> usize {
    QUIET.load(Ordering::Acquire)
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
            .args(SArgs {
                a0: mask,
                a1: base,
                ..Default::default()
            })
            .call();
    }
}
