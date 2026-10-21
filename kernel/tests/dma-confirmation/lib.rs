#![cfg(test)]
#![allow(dead_code)]

use std::cell::Cell;
use std::sync::Mutex;

#[path = "fixtures/references.rs"]
mod references;
#[path = "fixtures/hold.rs"]
mod hold;
#[path = "fixtures/pend.rs"]
mod pend;
#[path = "fixtures/task.rs"]
mod task;
#[path = "fixtures/tick.rs"]
mod tick;
#[path = "../health/dma_confirmation.rs"]
mod fixture;
#[path = "../../../programs/src/harness/probe/device_confirmation/injection.rs"]
mod injection;
#[path = "../../../programs/src/driver/iommu/api.rs"]
mod iommu;
#[path = "../../../programs/src/driver/recovery/api.rs"]
mod recovery;

static SERIAL: Mutex<()> = Mutex::new(());
thread_local! {
    static REFERENCES: Cell<usize> = const { Cell::new(2) };
}

fn testing() -> bool {
    true
}

#[macro_export]
macro_rules! putln {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

mod core {
    pub mod work {
        pub mod room {
            pub mod scheduler {
                pub mod roster {
                    pub fn muster(id: env::TaskId) -> Option<crate::task::Task> {
                        assert_eq!(id, env::TaskId::new(200));
                        Some(crate::task::Task {
                            pend: crate::pend::Pend { references: crate::references::References },
                        })
                    }
                }
            }
        }
    }
}

mod machine {
    pub mod clock {
        pub fn now() -> crate::tick::Tick {
            crate::tick::Tick
        }
    }
}

fn configure(enabled: bool, op: u8) {
    let root = env::TaskId::new(41);
    assert!(!fixture::reject(
        root, root, true, &injection::command(enabled, 100, 200, u64::from(op), 1, 200),
    ));
}

fn request(op: u8) -> Vec<u8> {
    use env::wire::Span;
    let mut bytes = vec![0u8; iommu::Step::LEN];
    let count = iommu::Step { op, serial: 1, reason: 0, back: env::PieToken::NONE }
        .store_at(&mut bytes, 0).unwrap();
    bytes.truncate(count);
    bytes
}

fn reject(bytes: &[u8]) -> bool {
    fixture::reject(env::TaskId::new(200), env::TaskId::new(100), false, bytes)
}

#[test]
fn real_iommu_and_recovery_frames_match_without_assuming_buffer_capacity_is_wire_length() {
    use env::wire::Span;
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    for op in (iommu::INVALIDATE..=iommu::RELEASE).step_by(2) {
        configure(true, op);
        let bytes = request(op);
        assert_eq!(bytes.len(), 18);
        assert!(reject(&bytes));
        assert!(reject(&bytes), "failure persists until explicit removal");
        configure(false, op);
        assert!(!reject(&bytes));
    }
    configure(true, recovery::STOP);
    let mut bytes = [0u8; recovery::Step::LEN];
    let count = recovery::Step {
        op: recovery::STOP, serial: 1, back: env::PieToken::NONE,
    }.store_at(&mut bytes, 0).unwrap();
    assert_eq!(count, 17);
    assert!(reject(&bytes[..count]));
    configure(false, recovery::STOP);
}

#[test]
fn caller_target_operation_serial_and_minimum_length_are_all_required() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    configure(true, iommu::RELEASE);
    let bytes = request(iommu::RELEASE);
    assert!(!fixture::reject(env::TaskId::new(201), env::TaskId::new(100), false, &bytes));
    assert!(!fixture::reject(env::TaskId::new(200), env::TaskId::new(101), false, &bytes));
    assert!(!reject(&request(iommu::INVALIDATE)));
    assert!(!reject(&bytes[..8]));
    let mut stale = bytes.clone();
    stale[1..9].copy_from_slice(&2u64.to_le_bytes());
    assert!(!reject(&stale));
    assert!(reject(&bytes));
    configure(false, iommu::RELEASE);
}

#[test]
fn nonroot_or_other_owned_command_cannot_remove_injection() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    configure(true, iommu::RELEASE);
    let bytes = injection::command(false, 100, 200, u64::from(iommu::RELEASE), 1, 200);
    let root = env::TaskId::new(41);
    assert!(!fixture::reject(root, root, false, &bytes));
    assert!(!fixture::reject(env::TaskId::new(42), root, true, &bytes));
    assert!(reject(&request(iommu::RELEASE)));
    configure(false, iommu::RELEASE);
}

#[test]
fn missing_dma_reference_is_a_failure_not_a_successful_retention_proof() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    configure(true, iommu::RELEASE);
    REFERENCES.with(|count| count.set(1));
    assert!(std::panic::catch_unwind(|| reject(&request(iommu::RELEASE))).is_err());
    REFERENCES.with(|count| count.set(2));
    configure(false, iommu::RELEASE);
}

#[test]
fn reference_failure_reaches_the_primitive_and_keeps_the_same_handle() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    let root = env::TaskId::new(41);
    let command = |enabled| injection::command(
        enabled, 100, 200, injection::REFERENCE_RELEASE, 1, 200,
    );
    assert!(!fixture::reject(root, root, true, &command(true)));
    assert!(!reject(&request(iommu::RELEASE)), "RPC delivery must remain real");
    assert!(!fixture::reject_reference(env::TaskId::new(201), 777));
    assert!(!fixture::reject_reference(env::TaskId::new(200), 778));
    assert!(fixture::reject_reference(env::TaskId::new(200), 777));
    assert!(fixture::reject_reference(env::TaskId::new(200), 777));
    assert!(!fixture::reject(root, root, true, &command(false)));
    assert!(!fixture::reject_reference(env::TaskId::new(200), 777));
    REFERENCES.with(|count| count.set(1));
    fixture::reference_released(env::TaskId::new(200), 777);
    REFERENCES.with(|count| count.set(2));
}
