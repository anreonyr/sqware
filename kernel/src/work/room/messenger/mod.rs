mod doom;
mod handoff;
mod reap;
mod wait;

use reap::HUSKS;
use wait::holder::{holders, void};
use wait::site::{SITE_SHARDS, prune, shard_at};

static EXIT_REASON: [core::sync::atomic::AtomicUsize; crate::layout::MAX_HART_SLOTS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; crate::layout::MAX_HART_SLOTS];

pub(crate) fn set_exit_reason(reason: usize) {
    let slot = crate::hart::hart_id()
        .get()
        .min(crate::layout::MAX_HART_SLOTS - 1);
    EXIT_REASON[slot].store(reason, core::sync::atomic::Ordering::Relaxed);
}

fn take_exit_reason() -> usize {
    let slot = crate::hart::hart_id()
        .get()
        .min(crate::layout::MAX_HART_SLOTS - 1);
    EXIT_REASON[slot].swap(0, core::sync::atomic::Ordering::Relaxed)
}

static EXIT_NOTE: [core::sync::atomic::AtomicUsize; crate::layout::MAX_HART_SLOTS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; crate::layout::MAX_HART_SLOTS];
static EXIT_NOTE_LEN: [core::sync::atomic::AtomicUsize; crate::layout::MAX_HART_SLOTS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; crate::layout::MAX_HART_SLOTS];

pub(crate) fn set_exit_note(va: usize, len: usize) {
    let slot = crate::hart::hart_id()
        .get()
        .min(crate::layout::MAX_HART_SLOTS - 1);
    EXIT_NOTE[slot].store(va, core::sync::atomic::Ordering::Relaxed);
    EXIT_NOTE_LEN[slot].store(len, core::sync::atomic::Ordering::Relaxed);
}

fn take_exit_note() -> (usize, usize) {
    let slot = crate::hart::hart_id()
        .get()
        .min(crate::layout::MAX_HART_SLOTS - 1);
    let va = EXIT_NOTE[slot].swap(0, core::sync::atomic::Ordering::Relaxed);
    let len = EXIT_NOTE_LEN[slot].swap(0, core::sync::atomic::Ordering::Relaxed);
    (va, len)
}

pub(crate) use env::EXIT_FAULT;

pub(crate) const EXIT_DOOM: usize = 0xFFFF_FFFE;

pub(crate) const EXIT_CASCADE: usize = 0xFFFF_FFFD;

pub(crate) use doom::{branch_stats, cull, doom, sweep_doomed, take_doomed};
pub(crate) use handoff::Handoff;
pub(crate) use reap::{hook, quit};
pub(crate) use wait::holder::Ticket;
#[cfg(debug_assertions)]
pub(crate) use wait::site::FWD_MAX;
pub(crate) use wait::site::WakeKey;
#[cfg(debug_assertions)]
pub(crate) use wait::site_count;
pub(crate) use wait::{
    fall, forward, join, knock, park, park_until, redeem, signal, unforward, wait, wake, wipe,
    wipe_space,
};

pub(crate) fn rip() {
    for shard in 0..SITE_SHARDS {
        let sites_out = core::mem::take(&mut *shard_at(shard).lock());
        drop(sites_out);
    }
    let husks_out = HUSKS.lock().take();
    drop(husks_out);
    holders().lock().clear();
    doom::rip();
}

#[cfg(debug_assertions)]
pub(crate) fn probe_bookkeeping() -> (usize, usize) {
    let holders_n = holders().lock().len();
    let husks_n = HUSKS.lock().len();
    (holders_n, husks_n)
}
