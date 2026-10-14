use serde::Serialize;

mod boot;
mod env;
mod halt;
mod memory;
mod room;

pub use boot::BootEvent;
pub use env::EnvEvent;
pub use halt::HaltEvent;
pub use memory::MemoryEvent;
pub use room::RoomEvent;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Room(RoomEvent),
    Env(EnvEvent),
    Memory(MemoryEvent),
    Halt(HaltEvent),
    Boot(BootEvent),
}
