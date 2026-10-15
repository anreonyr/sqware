use crate::retention::{Close, DeviceRange, Failure, Holds, Phase, Registration, Scope};
use crate::{accessor::Accessor, platform::Device, references::References};
use env::{AcquireSource, FlatReference, PieToken, ReferenceRange, TaskId};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

fn range(base: u64) -> DeviceRange {
    DeviceRange::new(base, 4096).unwrap()
}
fn cap() -> PieToken {
    PieToken::from_bytes(&3usize.to_le_bytes()).unwrap()
}
fn source() -> AcquireSource {
    AcquireSource::Reference(
        ReferenceRange::new(
            FlatReference::from_bytes(&1usize.to_le_bytes()).unwrap(),
            0,
            1,
        )
        .unwrap(),
    )
}

fn registration(range: DeviceRange) -> Registration {
    Registration {
        range,
        source: source(),
        retention: cap(),
    }
}

fn fixture() -> (Rc<RefCell<Vec<u8>>>, References, Rc<RefCell<Accessor>>) {
    let ram = Rc::new(RefCell::new(vec![0; 4096]));
    let references = References {
        source: Rc::downgrade(&ram),
        held: BTreeMap::new(),
        next: 1,
        fail_acquire: false,
        fail_release: false,
    };
    let accessor = Rc::new(RefCell::new(Accessor {
        ram: Rc::downgrade(&ram),
        active: false,
        timeout: false,
        pending: 0,
        synchronized: 0,
        isolated: 0,
    }));
    (ram, references, accessor)
}

fn attach(holds: &mut Holds, accessor: &Rc<RefCell<Accessor>>, base: u64, task: usize) {
    holds
        .attach(Scope::new(
            range(base),
            TaskId::new(task),
            Some(Box::new(Device {
                range: range(base),
                accessor: accessor.clone(),
            })),
        ))
        .unwrap();
}

#[test]
fn independent_accessor_survives_source_loss_death_and_isolation_timeout() {
    let (ram, mut references, accessor) = fixture();
    let mut holds = Holds::new();
    attach(&mut holds, &accessor, 0x1000, 7);
    assert_eq!(
        holds.publish(1, |_| panic!("unregistered")),
        Err(Failure::Unknown)
    );
    let id = holds
        .register(registration(range(0x1000)), &mut references)
        .unwrap();
    assert!(!accessor.borrow().active);
    holds
        .publish(id, |_| {
            accessor.borrow_mut().active = true;
            Ok(())
        })
        .unwrap();
    drop(ram);
    // Ordinary source cleanup happened before any death notification.
    accessor.borrow().write(0x41).unwrap();
    assert_eq!(references.held.values().next().unwrap().borrow()[0], 0x41);
    holds.retry(&mut references);
    assert_eq!(references.held.len(), 1, "lost notification is not release");
    holds.offline(TaskId::new(7));
    holds.offline(TaskId::new(7));
    assert_eq!(
        holds.publish(id, |_| panic!("closed")),
        Err(Failure::Closed)
    );
    accessor.borrow_mut().timeout = true;
    assert!(holds.retry(&mut references), "new failure is reportable");
    assert_eq!(holds.occupied().next().unwrap().4, Some(Failure::Pending));
    accessor.borrow().write(0x42).unwrap();
    assert_eq!(references.held.len(), 1);
    accessor.borrow_mut().timeout = false;
    accessor.borrow_mut().pending = 1;
    holds.retry(&mut references);
    assert_eq!(references.held.len(), 1, "in-flight access must finish too");
    accessor.borrow_mut().pending = 0;
    holds.retry(&mut references);
    assert_eq!(accessor.borrow().write(0x43), Err(Failure::Closed));
    assert!(references.held.is_empty());
    assert!(
        references.source.upgrade().is_none(),
        "last actual RAM owner retired"
    );
    holds.offline(TaskId::new(7));
    holds.retry(&mut references);
    assert_eq!(
        holds.attach(Scope::new(range(0x1000), TaskId::new(8), None)),
        Err(Failure::Conflict)
    );
}

#[test]
fn normal_sync_and_release_failure_retry_do_not_reopen_or_repeat_sync() {
    let (ram, mut references, accessor) = fixture();
    let mut holds = Holds::new();
    attach(&mut holds, &accessor, 0x1000, 7);
    // Every work-memory category uses the same registration rule; no device-count bound.
    for _ in 0..128 {
        let id = holds
            .register(registration(range(0x1000)), &mut references)
            .unwrap();
        holds
            .publish(id, |_| {
                accessor.borrow_mut().active = true;
                Ok(())
            })
            .unwrap();
    }
    drop(ram);
    holds.close(range(0x1000), Close::Normal).unwrap();
    references.fail_release = true;
    holds.retry(&mut references);
    assert_eq!(accessor.borrow().synchronized, 1);
    assert_eq!(accessor.borrow().isolated, 0);
    assert_eq!(holds.occupied().next().unwrap().2, Phase::Stopped);
    assert_eq!(references.held.len(), 128);
    assert_eq!(
        holds.register(registration(range(0x1000)), &mut references),
        Err(Failure::Closed)
    );
    references.fail_release = false;
    holds.retry(&mut references);
    assert_eq!(accessor.borrow().synchronized, 1);
    assert!(references.held.is_empty());
}

#[test]
fn partial_publish_failure_requires_real_isolation() {
    let (ram, mut references, accessor) = fixture();
    let mut holds = Holds::new();
    attach(&mut holds, &accessor, 0x1000, 7);
    let id = holds
        .register(registration(range(0x1000)), &mut references)
        .unwrap();
    drop(ram);
    assert_eq!(
        holds.publish(id, |_| {
            accessor.borrow_mut().active = true;
            Err(Failure::Pending)
        }),
        Err(Failure::Pending)
    );
    accessor.borrow_mut().timeout = true;
    holds.retry(&mut references);
    accessor.borrow().write(9).unwrap();
    assert_eq!(references.held.len(), 1);
    accessor.borrow_mut().timeout = false;
    holds.retry(&mut references);
    assert!(!accessor.borrow().active);
    assert!(references.held.is_empty());
}

#[test]
fn missing_provider_never_publishes_and_close_reclaims_only_unpublished() {
    let (_ram, mut references, _) = fixture();
    let mut holds = Holds::new();
    holds
        .attach(Scope::new(range(0x1000), TaskId::new(7), None))
        .unwrap();
    let id = holds
        .register(registration(range(0x1000)), &mut references)
        .unwrap();
    assert_eq!(
        holds.publish(id, |_| panic!("no provider")),
        Err(Failure::Unavailable)
    );
    holds.retry(&mut references);
    assert_eq!(
        references.held.len(),
        1,
        "open registered memory is not cleanup debt"
    );
    holds.close_all();
    holds.retry(&mut references);
    assert!(references.held.is_empty());
    assert_eq!(
        holds.register(registration(range(0x1000)), &mut references),
        Err(Failure::Closed)
    );
    assert_eq!(
        holds.attach(Scope::new(range(0x2000), TaskId::new(8), None)),
        Err(Failure::Closed)
    );
}

#[test]
fn range_binding_authorization_and_acquire_failure_leave_no_unowned_reference() {
    let (_ram, mut references, accessor) = fixture();
    let mut holds = Holds::new();
    assert_eq!(
        holds.attach(Scope::new(
            range(0x1000),
            TaskId::new(7),
            Some(Box::new(Device {
                range: range(0x2000),
                accessor: accessor.clone(),
            }))
        )),
        Err(Failure::Denied)
    );
    attach(&mut holds, &accessor, 0x1000, 7);
    assert_eq!(
        holds.attach(Scope::new(
            DeviceRange::new(0x1800, 4096).unwrap(),
            TaskId::new(8),
            None
        )),
        Err(Failure::Conflict)
    );
    assert!(DeviceRange::new(u64::MAX, 2).is_none());
    assert!(DeviceRange::new(1, 0).is_none());
    assert_eq!(
        holds.register(
            Registration {
                range: range(0x1000),
                source: source(),
                retention: PieToken::NONE,
            },
            &mut references
        ),
        Err(Failure::Denied)
    );
    references.fail_acquire = true;
    assert_eq!(
        holds.register(registration(range(0x1000)), &mut references),
        Err(Failure::Denied)
    );
    assert!(references.held.is_empty());
    assert_eq!(holds.occupied().count(), 0);
}

#[test]
fn failed_scope_does_not_freeze_unrelated_scope_and_fault_escalates_normal() {
    let (_ram, mut references, first) = fixture();
    let (_, _, second) = fixture();
    let mut holds = Holds::new();
    attach(&mut holds, &first, 0x1000, 7);
    attach(&mut holds, &second, 0x3000, 8);
    for (range, accessor) in [(range(0x1000), &first), (range(0x3000), &second)] {
        let id = holds
            .register(registration(range), &mut references)
            .unwrap();
        holds
            .publish(id, |_| {
                accessor.borrow_mut().active = true;
                Ok(())
            })
            .unwrap();
    }
    first.borrow_mut().timeout = true;
    holds.close_all();
    holds.offline(TaskId::new(7));
    holds.retry(&mut references);
    assert_eq!(references.held.len(), 1);
    assert_eq!(first.borrow().isolated, 1);
    assert_eq!(first.borrow().synchronized, 0);
    assert_eq!(second.borrow().synchronized, 1);
    assert_eq!(holds.occupied().next().unwrap().0, range(0x1000));
}
