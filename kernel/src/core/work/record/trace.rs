use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};
use serde::Serialize;

use alloc::alloc::{Allocator, Layout};
use fack::prelude::Error;

use crate::core::memory::allocator::spare;
use crate::core::memory::manager::FaultKind;
use crate::machine::hart;
use crate::machine::sync::OnceLock;

pub const BUFFER_SIZE: usize = 512;
pub const TRACE_DUMP: usize = 64;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Room(RoomEvent),
    Env(EnvEvent),
    Memory(MemoryEvent),
    Halt(HaltEvent),
    Boot(BootEvent),
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEvent {
    Spawn {
        tid: usize,
    },
    Switch {
        prev_tid: usize,
        next_tid: usize,
    },
    Starve {
        tid: usize,
    },
    Park {
        tid: usize,
        wake_at: usize,
    },
    Wait {
        tid: usize,
        key: usize,
    },
    Wake {
        tid: usize,
    },
    Exit {
        tid: usize,
        reason: usize,
    },
    Reap {
        tid: usize,
    },
    FaultKilled {
        tid: usize,
        cause: usize,
        stval: usize,
    },
    Doomed {
        tid: usize,
        by: usize,
    },
    Idle,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvEvent {
    Call { call: usize, arg: usize },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEvent {
    PageFault {
        va: usize,
        fault: FaultKind,
        resolved: bool,
    },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltEvent {
    Halt,
    Panic,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootEvent {
    Launch { hart: usize },
    Done { hart: usize },
}

#[derive(Clone, Copy, Serialize)]
pub struct Event {
    pub(crate) when: u64,
    pub(crate) kind: EventKind,
}

impl Event {
    const EMPTY: Event = Event {
        when: 0,
        kind: EventKind::Room(RoomEvent::Idle),
    };
}

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

pub fn dump<F: FnMut(&Event)>(hart: usize, k: usize, mut f: F) {
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
    let w = t.cursor.load(Ordering::Relaxed);
    let start = w.saturating_sub(k);
    for i in start..w {
        f(unsafe { &*t.buffer.as_ptr().add(i % BUFFER_SIZE) });
    }
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

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceInitError {
    #[error("spare ring allocation failed")]
    OutOfMemory,
    #[error("trace already initialized")]
    AlreadyInit,
}

pub fn hart_rows() -> usize {
    (TRACE_DUMP / hart::hart_count()).max(1)
}
