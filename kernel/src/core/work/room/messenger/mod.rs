mod doom;
mod handoff;
mod reap;
mod wait;

use core::sync::atomic::{AtomicUsize, Ordering};

use reap::HUSKS;
use wait::holder::{holders, void};
use wait::site::{SITE_SHARDS, prune, shard_at};

struct ExitNote {
    reason: AtomicUsize,
    va: AtomicUsize,
    len: AtomicUsize,
}

impl ExitNote {
    const fn new() -> Self {
        Self {
            reason: AtomicUsize::new(0),
            va: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
        }
    }
}

static EXIT: [ExitNote; crate::machine::layout::MAX_HART_SLOTS] =
    [const { ExitNote::new() }; crate::machine::layout::MAX_HART_SLOTS];

fn slot() -> usize {
    crate::machine::hart::hart_id()
        .get()
        .min(crate::machine::layout::MAX_HART_SLOTS - 1)
}

pub(crate) fn set_exit_reason(reason: usize) {
    EXIT[slot()].reason.store(reason, Ordering::Relaxed);
}

fn take_exit_reason() -> usize {
    EXIT[slot()].reason.swap(0, Ordering::Relaxed)
}

pub(crate) fn set_exit_note(va: usize, len: usize) {
    let note = &EXIT[slot()];
    note.va.store(va, Ordering::Relaxed);
    note.len.store(len, Ordering::Relaxed);
}

fn take_exit_note() -> (usize, usize) {
    let note = &EXIT[slot()];
    (
        note.va.swap(0, Ordering::Relaxed),
        note.len.swap(0, Ordering::Relaxed),
    )
}

pub(crate) use env::EXIT_FAULT;

pub(crate) const EXIT_DOOM: usize = 0xFFFF_FFFE;

pub(crate) const EXIT_CASCADE: usize = 0xFFFF_FFFD;

pub(crate) use doom::{branch_stats, cull, doom, slay, take_doomed};
pub(crate) use handoff::Handoff;
pub(crate) use reap::{hook, quit};
pub(crate) use wait::holder::Ticket;
#[cfg(debug_assertions)]
pub(crate) use wait::fwd::FWD_MAX;
pub(crate) use wait::key::WakeKey;
#[cfg(debug_assertions)]
pub(crate) use wait::site_count;
pub(crate) use wait::{
    fall, forward, knock, park, park_until, redeem, signal, unforward, wait, wake, wipe, wipe_space,
};

pub(crate) fn rip() {
    for shard in 0..SITE_SHARDS {
        let sites_out = core::mem::take(&mut *shard_at(shard).lock());
        drop(sites_out);
    }
    let husks_out = HUSKS.lock().take();
    drop(husks_out);
    holders().lock().clear();
}

#[cfg(debug_assertions)]
pub(crate) fn probe_bookkeeping() -> (usize, usize) {
    let holders_n = holders().lock().len();
    let husks_n = HUSKS.lock().len();
    (holders_n, husks_n)
}

pub(crate) use wait::unit_join;

pub(crate) use wait::call::WaitCall;
pub(crate) use wait::mail_await;
