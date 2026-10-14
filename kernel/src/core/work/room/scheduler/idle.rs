//! 空着的核：谁睡着了记在位图上，活往哪放、怎么把核叫醒。

use core::sync::atomic::{AtomicUsize, Ordering};

use sbi::ecall::SArgs;
use sbi::{self, fid};

use crate::machine::hart::{self, HartId};

/// 每颗 hart 一位：置着 = 它正睡在空闲环里。
static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];

/// 挑核时转着圈从不同的位起，免得总挑到同一颗。
static PICK_CURSOR: AtomicUsize = AtomicUsize::new(0);

const WAITING_WORDS: usize = crate::machine::layout::MAX_HART_SLOTS / usize::BITS as usize;

pub(crate) fn sleep(hart: HartId) {
    debug_assert!(
        hart.get() < crate::machine::layout::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_or(bit, Ordering::AcqRel);
}

pub(crate) fn wake(hart: HartId) {
    debug_assert!(
        hart.get() < crate::machine::layout::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_and(!bit, Ordering::AcqRel);
}

pub(super) fn waiting(hart: HartId) -> bool {
    debug_assert!(
        hart.get() < crate::machine::layout::MAX_HART_SLOTS,
        "waiting hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].load(Ordering::Acquire) & bit != 0
}

/// 挑一颗睡着的核；一颗睡的也没有就轮着挑（不挑自己，除非只有自己）。
pub(super) fn pick() -> HartId {
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

/// 把所有睡着的核都叫一遍（停机之前用）。
pub(crate) fn yell() {
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

/// 叫醒指定的那一核：它得重新看一眼自己手上的活。
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
