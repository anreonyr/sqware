#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

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
static SLOT_ID: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

#[cfg(debug_assertions)]
static SLOT_HART: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

#[cfg(debug_assertions)]
static SLOT_ORIGIN: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

#[cfg(debug_assertions)]
static NEXT: AtomicUsize = AtomicUsize::new(1);

#[cfg(debug_assertions)]
pub(crate) fn register(origin: Origin) -> usize {
    let id = NEXT.fetch_add(1, Relaxed);
    let hart = crate::machine::hart::hart_id().get();
    for i in 0..SLOTS {
        if SLOT_ID[i].compare_exchange(0, id, Relaxed, Relaxed).is_ok() {
            SLOT_HART[i].store(hart, Relaxed);
            SLOT_ORIGIN[i].store(origin as usize, Relaxed);
            return id;
        }
    }
    0
}

#[cfg(debug_assertions)]
pub(crate) fn release(id: usize) {
    if id == 0 {
        return;
    }
    for i in 0..SLOTS {
        if SLOT_ID[i].load(Relaxed) == id {
            SLOT_ID[i].store(0, Relaxed);
            return;
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) fn check_block_heldout() {
    let me = crate::machine::hart::hart_id().get();
    for origin in [Origin::Muster, Origin::Snapshot] {
        for i in 0..SLOTS {
            if SLOT_ID[i].load(Relaxed) != 0
                && SLOT_ORIGIN[i].load(Relaxed) == origin as usize
                && SLOT_HART[i].load(Relaxed) == me
            {
                panic!(
                    "[weak] 挂起自检：本核栈上仍有抄件（出身：{origin:?}）—— 跨挂起的弱引用会让外壳 \
                     永远归还不掉（`strong 0 weak 1`）"
                );
            }
        }
    }
}
