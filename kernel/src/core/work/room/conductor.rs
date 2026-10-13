use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::Counter;

use crate::machine::hart::{self, HartId};
use crate::machine::sync::OnceLock;
use sbi::ecall::SArgs;
use sbi::{self, fid};

static PUSHED: Counter = Counter::new();
static REAPED: Counter = Counter::new();
static ROOTED: AtomicBool = AtomicBool::new(false);

const BARRIER_REPORT_AT: usize = 20_000_000;

static HALTING: AtomicBool = AtomicBool::new(false);
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);

static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];

static PICK_CURSOR: AtomicUsize = AtomicUsize::new(0);

const WAITING_WORDS: usize = crate::machine::layout::MAX_HART_SLOTS / usize::BITS as usize;

pub(crate) fn push() {
    PUSHED.bump();
}

pub(crate) fn exit() {
    REAPED.bump();
}

pub(super) fn done() -> bool {
    if !ROOTED.load(Ordering::Acquire) {
        return false;
    }
    let pushed = PUSHED.get();
    pushed == 0 || REAPED.get() == pushed
}

pub(crate) fn counts() -> (usize, usize) {
    (PUSHED.get(), REAPED.get())
}

pub(crate) fn barrier() -> (usize, usize) {
    (
        HALT_ARRIVED.load(Ordering::Acquire),
        crate::machine::hart::hart_count(),
    )
}

pub(crate) fn rooted() {
    ROOTED.store(true, Ordering::Release);
}

pub(super) fn halt() -> ! {
    crate::core::memory::manager::asid::vacate();
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        yell();
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

type Hook = fn();

static HOOKS: OnceLock<(&'static [Hook], fn())> = OnceLock::new();

pub(crate) fn hook(hooks: &'static [Hook], waiting: fn()) {
    let _ = HOOKS.set((hooks, waiting));
}

fn hooked() {
    if let Some((hooks, _)) = HOOKS.get() {
        for h in hooks.iter() {
            h();
        }
    }
}

pub(super) fn sleep(hart: HartId) {
    debug_assert!(
        hart.get() < crate::machine::layout::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_or(bit, Ordering::AcqRel);
}

pub(super) fn wake(hart: HartId) {
    debug_assert!(
        hart.get() < crate::machine::layout::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_and(!bit, Ordering::AcqRel);
}

pub(crate) fn waiting(hart: HartId) -> bool {
    debug_assert!(
        hart.get() < crate::machine::layout::MAX_HART_SLOTS,
        "waiting hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].load(Ordering::Acquire) & bit != 0
}

pub(crate) fn pick() -> HartId {
    let seat = PICK_CURSOR.fetch_add(1, Ordering::Relaxed);
    let n = hart::hart_count();
    debug_assert!(n > 0, "pick with no hart");
    let sm = seat % (usize::BITS as usize);
    let mut best_rel = usize::MAX;
    let mut best_bit = 0usize;
    for (w, word) in WAITING.iter().enumerate() {
        let bits = word.load(Ordering::Acquire);
        if bits == 0 {
            continue;
        }
        let rot = bits.rotate_right(sm as u32);
        let rel = rot.trailing_zeros() as usize;
        let bit = w * (usize::BITS as usize) + (sm + rel) % (usize::BITS as usize);
        if rel < best_rel {
            best_rel = rel;
            best_bit = bit;
        }
    }

    if best_rel != usize::MAX {
        HartId::new(best_bit)
    } else {
        let me = hart::hart_id();
        let mut to = seat % n;
        if n > 1 && to == me.get() {
            to = (to + 1) % n;
        }
        HartId::new(to)
    }
}

static KICKS: Counter = Counter::new();
static FALLBACK: Counter = Counter::new();

pub(crate) fn note_kick_ipi() {
    KICKS.bump();
}

pub(crate) fn note_fallback() {
    FALLBACK.bump();
}

pub(crate) fn kick_stats() -> (usize, usize) {
    (KICKS.get(), FALLBACK.get())
}

pub(super) fn yell() {
    for (w, word) in WAITING.iter().enumerate() {
        let waiting = word.load(Ordering::Acquire);
        if waiting == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: waiting,
                a1: w * (usize::BITS as usize),
                ..Default::default()
            })
            .call();
    }
}

pub(crate) fn nudge(hart: HartId) {
    let (word, bit) = hart.bit();
    let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: bit,
            a1: word * (usize::BITS as usize),
            ..Default::default()
        })
        .call();
}
