#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering::{Acquire, Relaxed, Release}};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Origin {
    Roster,
    Holder,
    TeamTasks,
    Sire,
    Muster,
    Snapshot,
}

#[cfg(debug_assertions)]
const SLOTS: usize = 48;

#[cfg(debug_assertions)]
static RECORDS: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

#[cfg(debug_assertions)]
const ORIGINS: usize = Origin::Snapshot as usize + 1;

#[cfg(debug_assertions)]
pub(crate) fn register(origin: Origin) -> usize {
    let hart = crate::machine::hart::hart_id().get();
    let record = hart * ORIGINS + origin as usize + 1;
    for (i, slot) in RECORDS.iter().enumerate() {
        if slot.compare_exchange(0, record, Acquire, Relaxed).is_ok() {
            return i + 1;
        }
    }
    0
}

#[cfg(debug_assertions)]
pub(crate) fn release(slot: usize) {
    if slot == 0 {
        return;
    }
    RECORDS[slot - 1].store(0, Release);
}

#[cfg(debug_assertions)]
pub(crate) fn check_block_heldout() {
    let me = crate::machine::hart::hart_id().get();
    for slot in &RECORDS {
        let record = slot.load(Acquire);
        if record == 0 || (record - 1) / ORIGINS != me {
            continue;
        }
        let origin = match (record - 1) % ORIGINS {
            value if value == Origin::Muster as usize => Origin::Muster,
            value if value == Origin::Snapshot as usize => Origin::Snapshot,
            _ => continue,
        };
        panic!(
            "[weak] 挂起自检：本核栈上仍有抄件（出身：{origin:?}）—— 跨挂起的弱引用会让外壳 \
             永远归还不掉（`strong 0 weak 1`）"
        );
    }
}
