use alloc::collections::binary_heap::BinaryHeap;
use core::cmp::Reverse;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use riscv::register::time;

use crate::lock::{Level, SpinLock};
use crate::runtime::chrono::clock::{self, Instant};
use sbi::{TimerCall, ecall::SArgs, fid::Timer};

const NONE: u64 = u64::MAX;

pub const BLIND_MS: u64 = 100;

pub fn blind_ceiling() -> u64 {
    clock::duration_to_ticks(Duration::from_millis(BLIND_MS))
}

static LATE_N: AtomicU64 = AtomicU64::new(0);
static LATE_SUM: AtomicU64 = AtomicU64::new(0);
static LATE_MAX: AtomicU64 = AtomicU64::new(0);

static TOCK_N: AtomicU64 = AtomicU64::new(0);
static MUTE_N: AtomicU64 = AtomicU64::new(0);

static TICKS: AtomicU64 = AtomicU64::new(0);

struct TimerHeap {
    inner: SpinLock<TimerInner>,
    nearest: AtomicU64,
}

struct TimerInner {
    heap: BinaryHeap<Reverse<(u64, u64)>>,
}

static TIMER_HEAP: TimerHeap = TimerHeap {
    inner: SpinLock::new_level(
        Level::L3,
        TimerInner {
            heap: BinaryHeap::new(),
        },
    ),
    nearest: AtomicU64::new(NONE),
};

impl TimerHeap {
    fn peek_nearest(&self) -> u64 {
        self.nearest.load(Ordering::Acquire)
    }

    fn recompute_nearest(&self, i: &TimerInner) {
        let t = i.heap.iter().map(|e| e.0.0).min().unwrap_or(NONE);
        self.nearest.store(t, Ordering::Release);
    }
}

pub fn tick() -> u64 {
    TICKS.fetch_add(1, Ordering::Relaxed) + 1
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn beat(interval: u64) {
    let next = (time::read() as u64).wrapping_add(interval);
    TimerCall::new(Timer::SetTimer)
        .args(SArgs {
            a0: next as usize,
            ..Default::default()
        })
        .call()
        .unwrap();
}

pub fn beat_until(ceil: u64) {
    let delta = match due() {
        Some(t) => t.as_ticks().saturating_sub(time::read() as u64).min(ceil),
        None => ceil,
    };
    beat(delta);
}

pub fn tock(handle: u64, wake_at: u64) -> Result<(), ()> {
    let mut i = TIMER_HEAP.inner.lock();
    i.heap.try_reserve(1).map_err(|_| ())?;
    i.heap.push(Reverse((wake_at, handle)));
    TIMER_HEAP.recompute_nearest(&i);
    TOCK_N.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn mute(handle: u64) {
    let mut i = TIMER_HEAP.inner.lock();
    let before = i.heap.len();
    i.heap.retain(|Reverse((_, h))| *h != handle);
    if i.heap.len() != before {
        MUTE_N.fetch_add(1, Ordering::Relaxed);
    }
    TIMER_HEAP.recompute_nearest(&i);
}

pub fn due() -> Option<Instant> {
    let t = TIMER_HEAP.peek_nearest();
    (t != NONE).then_some(Instant::from_ticks(t))
}

pub fn drain(now: Instant, out: &mut [u64]) -> usize {
    let mut n = 0usize;
    let mut i = TIMER_HEAP.inner.lock();
    let now = now.as_ticks();
    while let Some(Reverse((t, _))) = i.heap.peek() {
        if *t > now || n >= out.len() {
            break;
        }
        let Reverse((wake_at, handle)) = i.heap.pop().expect("peeked non-empty heap entry");
        out[n] = handle;
        n += 1;
        let late = now.saturating_sub(wake_at);
        LATE_N.fetch_add(1, Ordering::Relaxed);
        LATE_SUM.fetch_add(late, Ordering::Relaxed);
        LATE_MAX.fetch_max(late, Ordering::Relaxed);
    }
    TIMER_HEAP.recompute_nearest(&i);
    n
}

pub fn late_stats() -> (u64, u64, u64) {
    (
        LATE_N.load(Ordering::Relaxed),
        LATE_MAX.load(Ordering::Relaxed),
        LATE_SUM.load(Ordering::Relaxed),
    )
}

pub fn tock_stats() -> (u64, u64) {
    (
        TOCK_N.load(Ordering::Relaxed),
        MUTE_N.load(Ordering::Relaxed),
    )
}