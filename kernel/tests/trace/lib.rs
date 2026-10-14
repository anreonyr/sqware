#![feature(allocator_api)]
#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/core/memory/manager/fault.rs"]
mod fault;

mod core {
    pub mod memory {
        pub mod manager {
            pub use crate::fault::FaultKind;
        }
        pub mod allocator {
            pub mod spare {
                use ::core::alloc::{AllocError, Allocator, Layout};
                use ::core::ptr::NonNull;
                use ::core::sync::atomic::{AtomicBool, Ordering};
                use alloc::alloc::Global;

                pub static FAIL_NEXT: AtomicBool = AtomicBool::new(false);
                pub struct Spare;
                static SPARE: Spare = Spare;

                pub fn spare() -> &'static Spare { &SPARE }

                unsafe impl Allocator for Spare {
                    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
                        if FAIL_NEXT.swap(false, Ordering::Relaxed) {
                            Err(AllocError)
                        } else {
                            Global.allocate(layout)
                        }
                    }

                    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
                        unsafe { Global.deallocate(ptr, layout) };
                    }
                }
            }
        }
    }
}

mod machine {
    pub mod sync {
        pub use std::sync::OnceLock;
    }
    pub mod hart {
        pub struct HartId;
        impl HartId { pub fn get(&self) -> usize { 0 } }
        pub fn hart_id() -> HartId { HartId }
        pub fn hart_count() -> usize { 1 }
    }
    pub mod clock {
        pub struct Instant;
        impl Instant { pub fn as_ticks(self) -> u64 { 123 } }
        pub fn now() -> Instant { Instant }
    }
}

#[path = "../../src/core/work/record/trace/mod.rs"]
pub mod trace;

#[cfg(test)]
mod tests {
    use super::trace::{self, EventKind, RoomEvent, TraceInitError};
    use ::core::sync::atomic::Ordering;

    #[cfg(feature = "semihosting")]
    static CAPTURED: std::sync::Mutex<Vec<(usize, u64)>> = std::sync::Mutex::new(Vec::new());

    #[cfg(feature = "semihosting")]
    fn capture(kind: EventKind, hart: usize, when: u64) {
        assert!(matches!(kind, EventKind::Room(RoomEvent::Spawn { tid: 7 })));
        assert_eq!(hart, 0);
        CAPTURED.lock().unwrap().push((hart, when));
    }

    fn init() -> Result<(), TraceInitError> {
        #[cfg(feature = "semihosting")]
        { trace::init(capture) }
        #[cfg(not(feature = "semihosting"))]
        { trace::init() }
    }

    #[test]
    fn allocation_failure_does_not_publish_ring_or_bind_sink() {
        let event = EventKind::Room(RoomEvent::Spawn { tid: 7 });
        crate::core::memory::allocator::spare::FAIL_NEXT.store(true, Ordering::Relaxed);
        assert_eq!(init(), Err(TraceInitError::OutOfMemory));
        trace::note(event);
        let mut before = 0;
        trace::dump(0, 1, |_| before += 1);
        assert_eq!(before, 0);
        #[cfg(feature = "semihosting")]
        assert!(CAPTURED.lock().unwrap().is_empty());

        init().unwrap();
        trace::note(event);
        let mut after = 0;
        trace::dump(0, 1, |record| {
            after += 1;
            assert_eq!(
                serde_json::to_value(record).unwrap(),
                serde_json::json!({
                    "when": 123,
                    "kind": {"room": {"spawn": {"tid": 7}}}
                }),
            );
        });
        assert_eq!(after, 1);
        #[cfg(feature = "semihosting")]
        assert_eq!(&*CAPTURED.lock().unwrap(), &[(0, 123)]);
        assert_eq!(init(), Err(TraceInitError::AlreadyInit));
    }

    #[test]
    fn event_variants_keep_wire_schema() {
        use super::trace::{BootEvent, EnvEvent, HaltEvent, MemoryEvent};
        use crate::fault::FaultKind;
        use serde_json::json;

        for (kind, expected) in [
            (EventKind::Room(RoomEvent::Spawn { tid: 1 }), json!({"room":{"spawn":{"tid":1}}})),
            (EventKind::Room(RoomEvent::Switch { prev_tid: 1, next_tid: 2 }),
                json!({"room":{"switch":{"prev_tid":1,"next_tid":2}}})),
            (EventKind::Room(RoomEvent::Starve { tid: 1 }), json!({"room":{"starve":{"tid":1}}})),
            (EventKind::Room(RoomEvent::Park { tid: 1, wake_at: 2 }),
                json!({"room":{"park":{"tid":1,"wake_at":2}}})),
            (EventKind::Room(RoomEvent::Wait { tid: 1, key: 2 }),
                json!({"room":{"wait":{"tid":1,"key":2}}})),
            (EventKind::Room(RoomEvent::Wake { tid: 1 }), json!({"room":{"wake":{"tid":1}}})),
            (EventKind::Room(RoomEvent::Exit { tid: 1, reason: 2 }),
                json!({"room":{"exit":{"tid":1,"reason":2}}})),
            (EventKind::Room(RoomEvent::Reap { tid: 1 }), json!({"room":{"reap":{"tid":1}}})),
            (EventKind::Room(RoomEvent::FaultKilled { tid: 1, cause: 2, stval: 3 }),
                json!({"room":{"fault_killed":{"tid":1,"cause":2,"stval":3}}})),
            (EventKind::Room(RoomEvent::Doomed { tid: 1, by: 2 }),
                json!({"room":{"doomed":{"tid":1,"by":2}}})),
            (EventKind::Room(RoomEvent::Idle), json!({"room":"idle"})),
            (EventKind::Env(EnvEvent::Call { call: 1, arg: 2 }),
                json!({"env":{"call":{"call":1,"arg":2}}})),
            (EventKind::Memory(MemoryEvent::PageFault { va: 1, fault: FaultKind::Instruction, resolved: true }),
                json!({"memory":{"page_fault":{"va":1,"fault":"Instruction","resolved":true}}})),
            (EventKind::Memory(MemoryEvent::PageFault { va: 1, fault: FaultKind::Load, resolved: false }),
                json!({"memory":{"page_fault":{"va":1,"fault":"Load","resolved":false}}})),
            (EventKind::Memory(MemoryEvent::PageFault { va: 1, fault: FaultKind::Store, resolved: true }),
                json!({"memory":{"page_fault":{"va":1,"fault":"Store","resolved":true}}})),
            (EventKind::Halt(HaltEvent::Halt), json!({"halt":"halt"})),
            (EventKind::Halt(HaltEvent::Panic), json!({"halt":"panic"})),
            (EventKind::Boot(BootEvent::Launch { hart: 1 }), json!({"boot":{"launch":{"hart":1}}})),
            (EventKind::Boot(BootEvent::Done { hart: 1 }), json!({"boot":{"done":{"hart":1}}})),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), expected);
        }
    }
}
