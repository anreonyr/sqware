use ::core::fmt;
use alloc::{format, string::String, vec, vec::Vec};

use crate::core::work::record::trace::{
    self, BootEvent, EnvEvent, Event, EventKind, HaltEvent, MemoryEvent, RoomEvent,
};
use crate::machine::hart;
use super::report::Report;

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
        EventKind::Memory(MemoryEvent::PageFault { va, fault, resolved }) => {
            write!(w, "pagefault va={va:#x} kind={:?} resolved={resolved}", fault)
        }
        EventKind::Halt(HaltEvent::Halt) => write!(w, "halt"),
        EventKind::Halt(HaltEvent::Panic) => write!(w, "panic"),
        EventKind::Boot(BootEvent::Launch { hart }) => write!(w, "launch hart {hart}"),
        EventKind::Boot(BootEvent::Done { hart }) => write!(w, "boot done hart {hart}"),
    }
}

pub(super) fn panic_dump(r: &mut Report) {
    for h in 0..hart::hart_count() {
        let mut rows: Vec<Vec<Option<String>>> = vec![vec![Some("t".into()), Some("event".into())]];
        trace::dump(h, trace::hart_rows(), |e| {
            let mut d = String::new();
            let _ = fmt_description(e, &mut d);
            rows.push(vec![Some(format!("{:#018x}", e.when)), Some(d)]);
        });
        r.paragraph("trace", Some(format!("[trace] hart {h}:")))
            .items
            .extend(rows);
    }
}
