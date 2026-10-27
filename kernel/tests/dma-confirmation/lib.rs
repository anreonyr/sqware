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
use kernel_test_api::device_confirmation as injection;
#[path = "../../../programs/src/driver/iommu/api/mod.rs"]
mod iommu;
#[path = "../../../programs/src/driver/pci/api.rs"]
mod pci;

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
        root, root, true, &injection::command(enabled, 100, 200, u64::from(op), 1, 200, 0),
    ));
}

fn request(op: u8) -> Vec<u8> {
    use env::wire::Span;
    let mut bytes = vec![0u8; iommu::Step::LEN];
    let count = iommu::Step { op, engine: env::TaskId::new(200), key: 77,
        hub: env::TaskId::new(99), serial: 1, reason: 0, back: env::PieToken::NONE }
        .store_at(&mut bytes, 0).unwrap();
    bytes.truncate(count);
    bytes
}

fn reject(bytes: &[u8]) -> bool {
    fixture::reject(env::TaskId::new(200), env::TaskId::new(100), false, bytes)
}

#[test]
fn real_iommu_frames_match_without_assuming_buffer_capacity_is_wire_length() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    for op in (iommu::INVALIDATE..=iommu::RELEASE).step_by(2) {
        configure(true, op);
        let bytes = request(op);
        assert_eq!(bytes.len(), 42);
        assert!(reject(&bytes));
        assert!(reject(&bytes), "failure persists until explicit removal");
        configure(false, op);
        assert!(!reject(&bytes));
    }
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
    for end in 0..bytes.len() { assert!(!reject(&bytes[..end])); }
    let mut stale = bytes.clone();
    stale[25..33].copy_from_slice(&2u64.to_le_bytes());
    assert!(!reject(&stale));
    assert!(reject(&bytes));
    configure(false, iommu::RELEASE);
}

#[test]
fn nonroot_or_other_owned_command_cannot_remove_injection() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    configure(true, iommu::RELEASE);
    let bytes = injection::command(false, 100, 200, u64::from(iommu::RELEASE), 1, 200, 0);
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
        enabled, 100, 200, injection::REFERENCE_RELEASE, 1, 200, 0,
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

#[test]
fn before_stop_gate_rejects_the_real_pci_disable_and_delivers_hub_coordination() {
    use env::wire::Span;
    use hub_api::coordination::{Release, RELEASE};
    const RECOVERY: usize = 100;
    const PCI: usize = 300;
    const HUB: usize = 400;
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    REFERENCES.with(|count| count.set(2));
    let root = env::TaskId::new(41);
    let configure = |enabled| {
        assert!(!fixture::reject(root, root, true,
            &injection::command(
                enabled, HUB, RECOVERY, injection::BEFORE_DEVICE_STOP, 1, 200, PCI,
            )));
    };
    let gate = |from, to, bytes: &[u8]| fixture::reject(
        env::TaskId::new(to), env::TaskId::new(from), false, bytes);
    let mut disable = [0u8; pci::Toggle::LEN];
    let len = pci::Toggle { op: pci::DISABLE, serial: 1, requester: 8, back: env::PieToken::NONE }
        .store_at(&mut disable, 0).unwrap();
    assert_eq!(len, 21, "buffer capacity is not wire length");
    let disable = &disable[..len];
    let mut release = [0u8; Release::LEN];
    let len = Release { op: RELEASE, hub: env::TaskId::new(HUB), serial: 1, key: 7,
        execution: env::TaskId::new(500), reason: 1, back: env::PieToken::NONE }
        .store_at(&mut release, 0).unwrap();
    assert_eq!(len, 42);
    configure(true);
    assert_eq!(fixture::hits(), 0);
    assert!(!fixture::armed());
    assert!(!gate(HUB, RECOVERY, &release), "Hub coordination starts before the gate");
    for (from, to) in [(RECOVERY + 1, PCI), (RECOVERY, PCI + 1)] {
        assert!(!gate(from, to, disable));
        assert!(!gate(HUB, RECOVERY, &release), "a foreign pair cannot arm the gate");
    }
    let mut enable = disable.to_vec();
    enable[0] = pci::ENABLE;
    assert!(!gate(RECOVERY, PCI, &enable));
    let mut neighbour = disable.to_vec();
    neighbour[1..9].copy_from_slice(&2u64.to_le_bytes());
    assert!(!gate(RECOVERY, PCI, &neighbour), "neighbour serial 2 is not blocked");
    for end in 0..disable.len() {
        assert!(!gate(RECOVERY, PCI, &disable[..end]));
    }
    assert!(!gate(RECOVERY, PCI, &release), "any 42-byte frame stays delivered here");
    assert!(!gate(HUB, RECOVERY, &release), "Hub->Driver Release stays delivered");
    assert_eq!(fixture::hits(), 0);
    assert!(!fixture::armed());

    assert!(gate(RECOVERY, PCI, disable), "the actual PCI Disable is rejected");
    assert_eq!(fixture::hits(), 1);
    assert!(fixture::armed());
    assert!(gate(RECOVERY, PCI, disable), "the same request stays rejected");
    assert_eq!(fixture::hits(), 2);
    assert!(!gate(HUB, RECOVERY, &release), "Hub coordination must stay delivered when armed");
    assert!(!gate(HUB, RECOVERY, &release));
    assert!(!gate(RECOVERY, PCI, &neighbour));
    assert!(!gate(RECOVERY, PCI, &enable));
    assert_eq!(fixture::hits(), 2, "nonmatching traffic cannot advance the receipt");

    configure(false);
    assert!(!fixture::armed());
    assert!(!gate(RECOVERY, PCI, disable), "removal restores the actual Disable");
    configure(true);
    assert_eq!(fixture::hits(), 0, "re-arming clears the first-hit receipt");
    assert!(!fixture::armed());
    assert!(gate(RECOVERY, PCI, disable));
    assert_eq!(fixture::hits(), 1);
    configure(false);
}
