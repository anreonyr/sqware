use serde::Serialize;

mod kind;
pub use kind::{BootEvent, EnvEvent, EventKind, HaltEvent, MemoryEvent, RoomEvent};

#[derive(Clone, Copy, Serialize)]
pub struct Event {
    pub(crate) when: u64,
    pub(crate) kind: EventKind,
}

impl Event {
    pub(super) const EMPTY: Event = Event {
        when: 0,
        kind: EventKind::Room(RoomEvent::Idle),
    };
}
