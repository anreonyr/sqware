//! Standalone host entry for the authority model and provider-owned API.
extern crate alloc;

pub use system_api::identity::{frame, grant, limits};
pub mod system {
    pub mod identity {
        pub use crate::book;
        pub use crate::{frame, grant, limits};
        pub use frame::vocab::*;
        pub use frame::*;
        pub use grant::Grant;
    }
}
#[path = "../../src/system/identity/service/answer.rs"]
pub mod answer;
#[path = "../../src/system/identity/book/mod.rs"]
pub mod book;

#[test]
fn found_replays_a_lost_reply_through_the_production_wire_and_service() {
    use env::{PieToken, TaskId};
    use system::identity::*;
    let installer = TaskId::new(1);
    let caller = TaskId::new(3);
    let authority = TaskId::new(2);
    let mut book = book::IdentityBook::new(authority, installer).unwrap();
    let principal = book.root();
    book.bind(installer, crate::book::BindingRequest {
        task: caller,
        install: Install::Authorized(Subject::new(principal, &[]).unwrap()),
    }).unwrap();
    let mut bytes = [0; MAX_FRAME];
    let mut first = None;
    for back in [11u64, 12, 13] {
        let len = Wire::Found(FoundId { value: 7 })
            .store(PieToken::from_bytes(&back.to_le_bytes()).unwrap(), &mut bytes).unwrap();
        let (wire, _) = Wire::take(&bytes[..len]).unwrap();
        let response = answer::answer(&mut book, answer::Request {
            context: answer::Context { from: caller, endpoint: PieToken::mint(1), grant: Grant::Found },
            wire,
        });
        match first {
            None => first = Some(response),
            Some(expected) => assert_eq!(response, expected),
        }
    }
    assert_eq!(first, Some(Reply::Coalition(CoalitionId::new(authority, 0))));
    assert_eq!(book.found(caller, FoundId { value: 8 }).unwrap().slot, 1);
}

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static FAIL_ALLOCATION: Cell<bool> = const { Cell::new(false) }; }
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_ALLOCATION.try_with(Cell::get).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(pointer, layout);
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if FAIL_ALLOCATION.try_with(Cell::get).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            unsafe { System.realloc(pointer, layout, size) }
        }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn without_allocating<T>(operation: impl FnOnce() -> T) -> T {
    FAIL_ALLOCATION.set(true);
    let result = operation();
    FAIL_ALLOCATION.set(false);
    result
}

#[test]
fn install_id_closes_derivation_retry_and_instance_scoped_retirement() {
    use env::TaskId;
    use system::identity::*;
    fn token(value: u8) -> env::PieToken {
        env::PieToken::from_bytes(&[value, 0, 0, 0, 0, 0, 0, 0]).unwrap()
    }
    fn request(from: TaskId, endpoint: env::PieToken, wire: Wire) -> answer::Request {
        answer::Request { context: answer::Context { from, endpoint, grant: Grant::for_wire(&wire) }, wire: Some(wire) }
    }
    let installer = TaskId::new(1);
    let target = TaskId::new(3);
    let mut book = book::IdentityBook::new(TaskId::new(2), installer).unwrap();
    let id = InstallId { value: 41 };
    let spec = InstallSpec { task: target, install: Install::Derive { parent: book.root() } };
    let entry = token(7);
    let denied = answer::answer(&mut book, request(TaskId::new(9), entry, Wire::Install(id, spec)));
    assert_eq!(denied, Reply::Fail(Fail::Denied));
    let lost = answer::answer(&mut book, request(installer, entry, Wire::Install(id, spec)));
    let Reply::Installation(first) = lost else { panic!("installation reply") };
    assert_eq!(first.state, InstallState::Installed);
    let replay = answer::answer(&mut book, request(installer, entry, Wire::Install(id, spec)));
    assert_eq!(replay, Reply::Installation(first));
    let inspected = answer::answer(&mut book, request(installer, entry, Wire::Inspect(id)));
    assert_eq!(inspected, Reply::Installation(first));

    let retired = answer::answer(&mut book, request(installer, entry, Wire::Retire(id)));
    assert_eq!(retired, Reply::Installation(InstallStatus { state: InstallState::Retired, binding: None, failure: None }));
    let other_entry = token(8);
    let other = InstallId { value: 41 };
    let second = answer::answer(&mut book, request(installer, other_entry, Wire::Install(other, spec)));
    let Reply::Installation(second) = second else { panic!("second installation reply") };
    assert_eq!(second.state, InstallState::Installed);
    assert_ne!(first.binding.unwrap().origin.principal, second.binding.unwrap().origin.principal);
    assert_eq!(answer::answer(&mut book, request(installer, entry, Wire::Retire(id)),), retired);
    assert_eq!(book.resolve(target), second.binding);
}

#[test]
fn install_id_records_failure_and_unseen_cancel_tombstones() {
    use env::TaskId;
    use system::identity::*;
    let entry = env::PieToken::from_bytes(&[4, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let installer = TaskId::new(1);
    let mut book = book::IdentityBook::new(TaskId::new(2), installer).unwrap();
    let id = InstallId { value: 5 };
    for value in 10..90 {
        let unauthorized = env::PieToken::from_bytes(&[value, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        let reply = answer::answer(&mut book, answer::Request { context: answer::Context { from: TaskId::new(9), endpoint: unauthorized, grant: Grant::Install }, wire: Some(Wire::Cancel(InstallId { value: u64::from(value) })) });
        assert_eq!(reply, Reply::Fail(Fail::Denied));
    }
    let cancelled = answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Cancel(id)) });
    assert_eq!(cancelled, Reply::Installation(InstallStatus { state: InstallState::Cancelled, binding: None, failure: None }));
    let spec = InstallSpec { task: TaskId::new(6), install: Install::Authorized(Subject::new(book.root(), &[]).unwrap()) };
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Install(id, spec)) }), Reply::Fail(Fail::Changed));

    let broken = InstallId { value: 6 };
    let bad = InstallSpec { task: TaskId::new(7), install: Install::Derive { parent: PrincipalId::new(TaskId::new(88), 9) } };
    let reply = answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Install(broken, bad)) });
    let Reply::Installation(status) = reply else { panic!("failure state") };
    assert_eq!(status.state, InstallState::Failed);
    assert_eq!(status.failure, Some(Fail::WrongAuthority));
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Inspect(broken)) }), Reply::Installation(status));

    let next = InstallId { value: 7 };
    let good = InstallSpec { task: TaskId::new(8), install: Install::Derive { parent: book.root() } };
    assert!(matches!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Install(next, good)) }), Reply::Installation(InstallStatus { state: InstallState::Installed, .. })));
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Inspect(broken)) }), Reply::Installation(InstallStatus { state: InstallState::Closed, binding: None, failure: None }));
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: entry, grant: Grant::Install }, wire: Some(Wire::Install(broken, bad)) }), Reply::Fail(Fail::Changed));
}

#[test]
fn identity_owns_named_entry_references_and_validates_the_identity_object() {
    use env::TaskId;
    use system::identity::*;
    let installer = TaskId::new(1);
    let authority = TaskId::new(2);
    let mut book = book::IdentityBook::new(authority, installer).unwrap();
    let object = IdentityObject::Principal(book.root());
    let name = IdentityName::new("shell").unwrap();
    let first = OperatorEntryRef { operator: TaskId::new(12), root: false, id: 0, revision: 7 };
    let install_entry = env::PieToken::from_bytes(&[3, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let public_entry = env::PieToken::from_bytes(&[2, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let associate = Wire::Associate(object, name, first);
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: TaskId::new(9), endpoint: install_entry, grant: Grant::Install }, wire: Some(associate) }), Reply::Fail(Fail::Denied));
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: install_entry, grant: Grant::Install }, wire: Some(associate) }), Reply::Unit);
    let query = Wire::Reference(object, name);
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: TaskId::new(77), endpoint: public_entry, grant: Grant::Reference }, wire: Some(query) }), Reply::Reference(Some(first)));

    let invalid = IdentityObject::Principal(PrincipalId::new(authority, 99));
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: install_entry, grant: Grant::Install }, wire: Some(Wire::Associate(invalid, name, first)) }), Reply::Fail(Fail::UnknownPrincipal));
    let replacement = OperatorEntryRef { revision: 8, ..first };
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: installer, endpoint: install_entry, grant: Grant::Install }, wire: Some(Wire::Associate(object, name, replacement)) }), Reply::Unit);
    assert_eq!(answer::answer(&mut book, answer::Request { context: answer::Context { from: TaskId::new(77), endpoint: public_entry, grant: Grant::Reference }, wire: Some(query) }), Reply::Reference(Some(replacement)));
}

#[test]
fn allocation_failures_leave_the_authority_unchanged() {
    use env::TaskId;
    use system::identity::*;
    let installer = TaskId::new(1);
    let authority = TaskId::new(2);
    assert!(matches!(
        without_allocating(|| book::IdentityBook::new(authority, installer)),
        Err(Fail::Full)
    ));
    let mut book = book::IdentityBook::new(authority, installer).unwrap();
    let p = book.root();
    let mut last = p;
    while let Ok(q) = without_allocating(|| book.derive(installer, p)) {
        last = q;
    }
    let q = book.derive(installer, p).unwrap();
    assert_eq!(q.slot, last.slot + 1);
    let task = TaskId::new(3);
    let subject = Subject::new(p, &[]).unwrap();
    assert_eq!(
        without_allocating(|| book.bind(
            installer,
            crate::book::BindingRequest {
                task: task,
                install: Install::Authorized(subject)
            }
        )),
        Err(Fail::Full)
    );
    assert!(book.resolve(task).is_none());
    book.bind(
        installer,
        crate::book::BindingRequest {
            task: task,
            install: Install::Authorized(subject),
        },
    )
    .unwrap();
    assert_eq!(without_allocating(|| book.found(task, FoundId { value: 1 })), Err(Fail::Full));
    let c = book.found(task, FoundId { value: 1 }).unwrap();
    assert_eq!(c.slot, 0);
    assert_eq!(without_allocating(|| book.found(task, FoundId { value: 1 })), Ok(c));
    assert_eq!(
        without_allocating(|| book.admit(
            task,
            crate::book::Membership {
                coalition: c,
                principal: p
            }
        )),
        Err(Fail::Full)
    );
    assert_eq!(book.amid(p, c), Ok(false));
    book.admit(
        task,
        crate::book::Membership {
            coalition: c,
            principal: p,
        },
    )
    .unwrap();
    book.bind(
        installer,
        crate::book::BindingRequest {
            task: task,
            install: Install::Authorized(Subject::new(p, &[c]).unwrap()),
        },
    )
    .unwrap();
    assert!(without_allocating(|| book.page::<PrincipalId>(PageTarget::Members(c), None)).is_ok());
    assert_eq!(
        without_allocating(|| book.expel(
            task,
            crate::book::Membership {
                coalition: c,
                principal: p
            }
        )),
        Ok(())
    );
    assert_eq!(without_allocating(|| book.waive(task)), Ok(()));
    assert_eq!(without_allocating(|| book.unbind(installer, task)), Ok(()));
}

#[test]
fn all_identity_faces_reject_every_other_action_without_mutation() {
    use env::TaskId;
    use system::identity::*;
    let installer = TaskId::new(1);
    let task = TaskId::new(3);
    let mut book = book::IdentityBook::new(TaskId::new(2), installer).unwrap();
    let p = book.root();
    let subject = Subject::new(p, &[]).unwrap();
    book.bind(
        installer,
        crate::book::BindingRequest {
            task: task,
            install: Install::Authorized(subject),
        },
    )
    .unwrap();
    let c = book.found(task, FoundId { value: 1 }).unwrap();
    let actions = [
        Wire::Resolve(task),
        Wire::Matches(task, Selector::Exact(p)),
        Wire::Same(task, task),
        Wire::Sire(p),
        Wire::Heir(p, p),
        Wire::Amid(p, c),
        Wire::Members(c, None),
        Wire::Memberships(p, None),
        Wire::Adopt(subject),
        Wire::Waive,
        Wire::Restrict(subject),
        Wire::Derive(p),
        Wire::Found(FoundId { value: 1 }),
        Wire::Admit(c, p),
        Wire::Expel(c, p),
        Wire::Bind(task, Install::Authorized(subject)),
        Wire::Unbind(task),
        Wire::Activate(task, CoalitionSet::new(&[c]).unwrap()),
        Wire::Install(InstallId { value: 1 }, InstallSpec { task, install: Install::Authorized(subject) }),
        Wire::Inspect(InstallId { value: 1 }),
        Wire::Cancel(InstallId { value: 1 }),
        Wire::Retire(InstallId { value: 1 }),
        Wire::Associate(IdentityObject::Principal(p), IdentityName::new("x").unwrap(), OperatorEntryRef { operator: TaskId::new(9), root: false, id: 1, revision: 0 }),
        Wire::Reference(IdentityObject::Principal(p), IdentityName::new("x").unwrap()),
    ];
    for face in Grant::ALL {
        for wire in actions {
            if Grant::for_wire(&wire) == face {
                continue;
            }
            let before = book.resolve(task);
            assert_eq!(
                answer::answer(
                    &mut book,
                    answer::Request {
                        context: answer::Context { from: installer, endpoint: env::PieToken::from_bytes(&[1, 0, 0, 0, 0, 0, 0, 0]).unwrap(), grant: face },
                        wire: Some(wire)
                    }
                ),
                Reply::Fail(Fail::Denied)
            );
            assert_eq!(book.resolve(task), before);
            assert_eq!(book.amid(p, c), Ok(false));
        }
    }
}

#[test]
fn adopt_keeps_the_origin_and_restrict_moves_it() {
    use env::TaskId;
    use system::identity::*;
    let installer = TaskId::new(1);
    let task = TaskId::new(3);
    let mut book = book::IdentityBook::new(TaskId::new(2), installer).unwrap();
    let p = book.derive(installer, book.root()).unwrap();
    book.bind(
        installer,
        crate::book::BindingRequest {
            task: task,
            install: Install::Authorized(Subject::new(p, &[]).unwrap()),
        },
    )
    .unwrap();
    let q = book.derive(task, p).unwrap();
    let narrowed = Subject::new(q, &[]).unwrap();
    assert_eq!(
        answer::answer(
            &mut book,
            answer::Request {
                context: answer::Context { from: task, endpoint: env::PieToken::from_bytes(&[1, 0, 0, 0, 0, 0, 0, 0]).unwrap(), grant: Grant::Adopt },
                wire: Some(Wire::Adopt(narrowed))
            }
        ),
        Reply::Unit
    );
    assert_eq!(book.resolve(task).unwrap().origin.principal, p);
    assert_eq!(book.resolve(task).unwrap().current.principal, q);
    assert_eq!(
        answer::answer(
            &mut book,
            answer::Request {
                context: answer::Context { from: task, endpoint: env::PieToken::from_bytes(&[1, 0, 0, 0, 0, 0, 0, 0]).unwrap(), grant: Grant::Restrict },
                wire: Some(Wire::Restrict(narrowed))
            }
        ),
        Reply::Unit
    );
    assert_eq!(book.resolve(task).unwrap().origin.principal, q);
}
