use core::fmt;
use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};
use serde::Serialize;

use alloc::alloc::{Allocator, Layout};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use fack::prelude::Error;

use crate::hart;
use crate::lock::OnceLock;
use crate::memory::allocator::spare;
use crate::memory::manager::fault::FaultKind;
use crate::runtime::diagnose::report::Report;

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
    when: u64,
    kind: EventKind,
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
        // SAFETY: i < BUFFER_SIZE；单生产者（本 hart 唯一写者），跨核读者由 halt 停写互斥
        unsafe { *(self.buffer.as_ptr() as *mut Event).add(i) = Event { when, kind } };
    }

    #[allow(unused)]
    fn clear(&self) {
        self.cursor.store(0, Ordering::Relaxed);
    }
}

static POOL: OnceLock<&'static [Trace]> = OnceLock::new();

pub fn ring_bytes(h: usize) -> usize {
    h * (size_of::<Trace>() + BUFFER_SIZE * size_of::<Event>())
}

pub fn note(kind: EventKind) {
    let Some(pool) = POOL.get() else {
        return;
    };
    let hart = hart::hart_id();
    let Some(t) = pool.get(hart.get()) else {
        return;
    };
    let when = crate::runtime::chrono::clock::now().as_ticks();
    t.note(kind, when);
    #[cfg(feature = "semihosting")]
    host_note(kind, hart.get(), when);
}

#[cfg(feature = "semihosting")]
fn host_note(kind: EventKind, hart: usize, when: u64) {
    use crate::runtime::diagnose::export::push;
    let e = Event { when, kind };
    if let Ok(json) = serde_json::to_vec(&HostEvent { h: hart, e: &e }) {
        push(&json);
    }
}

#[cfg(feature = "semihosting")]
#[derive(Serialize)]
struct HostEvent<'a> {
    h: usize,
    #[serde(flatten)]
    e: &'a Event,
}

pub fn dump<F: FnMut(&Event)>(hart: usize, k: usize, mut f: F) {
    let Some(t) = POOL.get().and_then(|p| p.get(hart)) else {
        return;
    };
    let w = t.cursor.load(Ordering::Relaxed);
    let start = w.saturating_sub(k);
    for i in start..w {
        // SAFETY: i % BUFFER_SIZE < BUFFER_SIZE；panic 期无写者
        f(unsafe { &*t.buffer.as_ptr().add(i % BUFFER_SIZE) });
    }
}

pub fn init() -> Result<(), TraceInitError> {
    let h = hart::hart_count();
    let total = ring_bytes(h);
    let layout = Layout::from_size_align(total, 16).expect("trace: ring layout");
    let chunk = spare::spare()
        .allocate(layout)
        .map_err(|_| TraceInitError::OutOfMemory)?;
    // SAFETY: chunk 为 spare 仓内块（16B 对齐）；下分窗口表区 + 事件槽区，互不重叠
    let base = chunk.as_ptr() as *mut u8 as usize;
    let traces = base as *mut Trace;
    let events = (base + h * size_of::<Trace>()) as *mut Event;
    for i in 0..h {
        // SAFETY: 槽区总长 = h × BUFFER_SIZE × size_of<Event>，本窗口切片在界内
        let buf =
            unsafe { core::slice::from_raw_parts_mut(events.add(i * BUFFER_SIZE), BUFFER_SIZE) };
        for slot in buf.iter_mut() {
            *slot = Event::EMPTY;
        }
        // SAFETY: traces 区内第 i 个 Trace 未初始化；boot 单核写入，无并发
        unsafe {
            traces.add(i).write(Trace {
                cursor: AtomicUsize::new(0),
                buffer: buf,
            });
        }
    }
    // SAFETY: 全部 h 个 Trace 已初始化；此后只读结构
    let pool = unsafe { core::slice::from_raw_parts(traces, h) };
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

fn fmt_description(e: &Event, w: &mut impl fmt::Write) -> fmt::Result {
    match e.kind {
        EventKind::Room(RoomEvent::Spawn { tid }) => write!(w, "spawn tid={tid}"),
        EventKind::Room(RoomEvent::Switch { prev_tid, next_tid }) => {
            write!(w, "switch {prev_tid}->{next_tid}")
        }
        EventKind::Room(RoomEvent::Starve { tid }) => write!(w, "starve tid={tid}"),
        EventKind::Room(RoomEvent::Park { tid, wake_at }) => {
            write!(w, "park tid={tid} @{wake_at:#x}")
        }
        EventKind::Room(RoomEvent::Wait { tid, key }) => {
            write!(w, "wait tid={tid} key={key:#x}")
        }
        EventKind::Room(RoomEvent::Wake { tid }) => write!(w, "wake tid={tid}"),
        EventKind::Room(RoomEvent::Exit { tid, reason }) => {
            write!(w, "exit tid={tid} reason={reason:#x}")
        }
        EventKind::Room(RoomEvent::Reap { tid }) => write!(w, "reap tid={tid}"),
        EventKind::Room(RoomEvent::FaultKilled { tid, cause, stval }) => {
            write!(w, "fault-killed tid={tid} cause={cause} stval={stval:#x}")
        }
        EventKind::Room(RoomEvent::Doomed { tid, by }) => {
            write!(w, "doomed tid={tid} by={by}")
        }
        EventKind::Room(RoomEvent::Idle) => write!(w, "idle"),
        EventKind::Env(EnvEvent::Call { call, arg }) => write!(w, "envcall #{call} arg={arg:#x}"),
        EventKind::Memory(MemoryEvent::PageFault {
            va,
            fault,
            resolved,
        }) => {
            write!(
                w,
                "pagefault va={va:#x} kind={:?} resolved={resolved}",
                fault
            )
        }
        EventKind::Halt(HaltEvent::Halt) => write!(w, "halt"),
        EventKind::Halt(HaltEvent::Panic) => write!(w, "panic"),
        EventKind::Boot(BootEvent::Launch { hart }) => write!(w, "launch hart {hart}"),
        EventKind::Boot(BootEvent::Done { hart }) => write!(w, "boot done hart {hart}"),
    }
}

pub fn hart_rows() -> usize {
    (TRACE_DUMP / hart::hart_count()).max(1)
}

pub fn panic_dump(r: &mut Report) {
    for h in 0..hart::hart_count() {
        let mut rows: Vec<Vec<Option<String>>> = vec![vec![Some("t".into()), Some("event".into())]];
        dump(h, hart_rows(), |e| {
            let mut d = String::new();
            let _ = fmt_description(e, &mut d);
            rows.push(vec![Some(format!("{:#018x}", e.when)), Some(d)]);
        });
        r.paragraph("trace", Some(format!("[trace] hart {h}:")))
            .items
            .extend(rows);
    }
}
