use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::alloc::{Allocator, Layout};

use crate::core::memory::allocator::spare;
use crate::machine::hart;
use crate::machine::sync::OnceLock;

mod error;
mod event;

pub use error::TraceInitError;
pub use event::{BootEvent, EnvEvent, Event, EventKind, HaltEvent, MemoryEvent, RoomEvent};

pub const BUFFER_SIZE: usize = 512;
pub const TRACE_DUMP: usize = 64;

pub struct Trace {
    cursor: AtomicUsize,
    buffer: &'static [Event],
}

impl Trace {
    fn note(&self, kind: EventKind, when: u64) {
        let i = self.cursor.fetch_add(1, Ordering::Relaxed) % BUFFER_SIZE;
        unsafe { *(self.buffer.as_ptr() as *mut Event).add(i) = Event { when, kind } };
    }

    #[allow(unused)]
    fn clear(&self) {
        self.cursor.store(0, Ordering::Relaxed);
    }

    fn dump<F: FnMut(&Event)>(&self, k: usize, mut f: F) {
        let w = self.cursor.load(Ordering::Relaxed);
        let start = w.saturating_sub(k);
        for i in start..w {
            f(unsafe { &*self.buffer.as_ptr().add(i % BUFFER_SIZE) });
        }
    }
}

#[cfg(not(feature = "semihosting"))]
static POOL: OnceLock<&'static [Trace]> = OnceLock::new();
#[cfg(feature = "semihosting")]
static POOL: OnceLock<(&'static [Trace], fn(EventKind, usize, u64))> = OnceLock::new();

pub fn ring_bytes(h: usize) -> usize {
    h * (size_of::<Trace>() + BUFFER_SIZE * size_of::<Event>())
}

pub fn note(kind: EventKind) {
    let Some(pool) = POOL.get() else {
        return;
    };
    #[cfg(not(feature = "semihosting"))]
    let traces = *pool;
    #[cfg(feature = "semihosting")]
    let (traces, sink) = *pool;
    let hart = hart::hart_id();
    let Some(t) = traces.get(hart.get()) else {
        return;
    };
    let when = crate::machine::clock::now().as_ticks();
    t.note(kind, when);
    #[cfg(feature = "semihosting")]
    sink(kind, hart.get(), when);
}

pub fn dump<F: FnMut(&Event)>(hart: usize, k: usize, f: F) {
    let Some(pool) = POOL.get() else {
        return;
    };
    #[cfg(not(feature = "semihosting"))]
    let traces = *pool;
    #[cfg(feature = "semihosting")]
    let traces = pool.0;
    let Some(t) = traces.get(hart) else {
        return;
    };
    t.dump(k, f);
}

pub fn init(
    #[cfg(feature = "semihosting")] sink: fn(EventKind, usize, u64),
) -> Result<(), TraceInitError> {
    let h = hart::hart_count();
    let total = ring_bytes(h);
    let layout = Layout::from_size_align(total, 16).expect("trace: ring layout");
    let chunk = spare::spare()
        .allocate(layout)
        .map_err(|_| TraceInitError::OutOfMemory)?;
    let base = chunk.as_ptr() as *mut u8 as usize;
    let traces = base as *mut Trace;
    let events = (base + h * size_of::<Trace>()) as *mut Event;
    for i in 0..h {
        let buf =
            unsafe { ::core::slice::from_raw_parts_mut(events.add(i * BUFFER_SIZE), BUFFER_SIZE) };
        for slot in buf.iter_mut() {
            *slot = Event::EMPTY;
        }
        unsafe {
            traces.add(i).write(Trace {
                cursor: AtomicUsize::new(0),
                buffer: buf,
            });
        }
    }
    let pool = unsafe { ::core::slice::from_raw_parts(traces, h) };
    #[cfg(feature = "semihosting")]
    let pool = (pool, sink);
    POOL.set(pool).map_err(|_| TraceInitError::AlreadyInit)?;
    Ok(())
}

pub fn hart_rows() -> usize {
    (TRACE_DUMP / hart::hart_count()).max(1)
}
