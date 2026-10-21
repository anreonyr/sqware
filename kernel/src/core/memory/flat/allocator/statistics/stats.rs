use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;

use crate::machine::sync::OnceLock;

#[cfg(debug_assertions)]
use super::block_kinds::{BLOCK_KIND_SHIFT, block_kind_at, note_block_kind};
use super::error::Error;
#[cfg(debug_assertions)]
use super::frame_kinds::{frame_kind_at, note_frame_kind};
#[cfg(debug_assertions)]
use super::kind::Kind;
#[cfg(debug_assertions)]
use super::kinds::Kinds;
#[cfg(debug_assertions)]
use super::mark::current;

struct Stats {
    frame_occupied: AtomicUsize,
    block_occupied: AtomicUsize,
    spare_occupied: AtomicUsize,
    spare_total: AtomicUsize,
    #[cfg(debug_assertions)]
    kinds: [AtomicUsize; Kind::COUNT],
}

static STATS: OnceLock<&'static Stats> = OnceLock::new();

fn stats() -> &'static Stats {
    STATS.get().expect("statistics not initialized")
}

pub fn init() -> Result<(), Error> {
    if STATS.get().is_some() {
        return Err(Error::AlreadyInitialized);
    }
    let s: &'static Stats = Box::leak(
        Box::try_new(Stats {
            frame_occupied: AtomicUsize::new(0),
            block_occupied: AtomicUsize::new(0),
            spare_occupied: AtomicUsize::new(0),
            spare_total: AtomicUsize::new(0),
            #[cfg(debug_assertions)]
            kinds: [const { AtomicUsize::new(0) }; Kind::COUNT],
        })
        .map_err(|_| Error::OutOfMemory)?,
    );
    STATS.set(s).map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}

pub(crate) fn record_frame_take(index: usize, power: usize) {
    let s = stats();
    s.frame_occupied.fetch_add(1, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    {
        let k = current();
        note_frame_kind(index, 1 << power, k);
        s.kinds[k.ix()].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (index, power);
}

pub(crate) fn record_frame_give(index: usize) {
    let s = stats();
    s.frame_occupied.fetch_sub(1, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    {
        let k = frame_kind_at(index);
        s.kinds[k.ix()].fetch_sub(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = index;
}

pub(crate) fn record_block_take(addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    {
        let k = if power < BLOCK_KIND_SHIFT {
            Kind::Plain
        } else {
            let k = current();
            note_block_kind(addr, power, k);
            k
        };
        stats().kinds[k.ix()].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (addr, power);
}

pub(crate) fn record_block_give(addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    {
        let k = block_kind_at(addr, power);
        stats().kinds[k.ix()].fetch_sub(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (addr, power);
}

pub(crate) fn record_pool_take() {
    stats().block_occupied.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_pool_give() {
    stats().block_occupied.fetch_sub(1, Ordering::Relaxed);
}

pub(crate) fn record_spare_take(bytes: usize) {
    stats().spare_occupied.fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_give(bytes: usize) {
    stats().spare_occupied.fetch_sub(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_total(total: usize) {
    records(|s| s.spare_total.store(total, Ordering::Relaxed));
}

pub fn frame_occupied() -> usize {
    stats().frame_occupied.load(Ordering::Relaxed)
}

pub fn block_occupied() -> usize {
    stats().block_occupied.load(Ordering::Relaxed)
}

#[cfg(debug_assertions)]
pub fn spare_occupied() -> usize {
    stats().spare_occupied.load(Ordering::Relaxed)
}

#[cfg(debug_assertions)]
pub fn spare_available() -> usize {
    let s = stats();
    s.spare_total
        .load(Ordering::Relaxed)
        .saturating_sub(s.spare_occupied.load(Ordering::Relaxed))
}

fn records(f: impl FnOnce(&Stats)) {
    if let Some(s) = STATS.get() {
        f(s);
    }
}

#[cfg(debug_assertions)]
pub(crate) fn kinds() -> Kinds {
    let s = stats();
    let mut out = [0usize; Kind::COUNT];
    for (i, c) in s.kinds.iter().enumerate() {
        out[i] = c.load(Ordering::Relaxed);
    }
    Kinds::new(out)
}
