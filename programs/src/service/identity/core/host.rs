//! Standalone host entry: real ABI and protocol types, no runtime dependency.
extern crate alloc;
extern crate self as protocol;

#[path = "../../../../../crates/protocol/src/common/table.rs"]
pub mod table;
pub mod common {
    pub use crate::table;
}

#[path = "../../../../../crates/protocol/src/wire/message.rs"]
pub mod message;
pub mod wire {
    pub use crate::message;
    pub const OK: u8 = 0;
}
#[path = "../../../../../crates/protocol/src/service/identity/limits.rs"]
pub mod limits;
#[path = "../../../../../crates/protocol/src/service/identity/grant.rs"]
pub mod grant;
#[path = "../../../../../crates/protocol/src/service/identity/frame/mod.rs"]
pub mod frame;
pub mod service {
    pub mod identity {
        pub use crate::book as core;
        pub use crate::{frame, grant, limits};
        pub use frame::*;
        pub use frame::vocab::*;
        pub use grant::Grant;
    }
}
#[path = "mod.rs"]
pub mod book;
#[path = "../serve/answer.rs"]
pub mod answer;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static FAIL_ALLOCATION: Cell<bool> = const { Cell::new(false) }; }
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_ALLOCATION.try_with(Cell::get).unwrap_or(false) { std::ptr::null_mut() }
        else { unsafe { System.alloc(layout) } }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout); }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if FAIL_ALLOCATION.try_with(Cell::get).unwrap_or(false) { std::ptr::null_mut() }
        else { unsafe { System.realloc(pointer, layout, size) } }
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
fn allocation_failures_leave_the_authority_unchanged() {
    use env::TaskId;
    use service::identity::*;
    let installer = TaskId::new(1);
    let authority = TaskId::new(2);
    assert!(matches!(without_allocating(|| book::IdentityBook::new(authority, installer)),
        Err(Fail::Full)));
    let mut book = book::IdentityBook::new(authority, installer).unwrap();
    let p = book.root();
    let mut last = p;
    while let Ok(q) = without_allocating(|| book.derive(installer, p)) { last = q; }
    let q = book.derive(installer, p).unwrap();
    assert_eq!(q.slot, last.slot + 1);
    let task = TaskId::new(3);
    let subject = Subject::new(p, &[]).unwrap();
    assert_eq!(without_allocating(|| book.bind(installer, task, Install::Authorized(subject))),
        Err(Fail::Full));
    assert!(book.resolve(task).is_none());
    book.bind(installer, task, Install::Authorized(subject)).unwrap();
    assert_eq!(without_allocating(|| book.found(task)), Err(Fail::Full));
    let c = book.found(task).unwrap();
    assert_eq!(c.slot, 0);
    assert_eq!(without_allocating(|| book.admit(task, c, p)), Err(Fail::Full));
    assert_eq!(book.amid(p, c), Ok(false));
    book.admit(task, c, p).unwrap();
    book.bind(installer, task, Install::Authorized(Subject::new(p, &[c]).unwrap())).unwrap();
    assert!(without_allocating(|| book.page::<PrincipalId>(PageTarget::Members(c), None)).is_ok());
    assert_eq!(without_allocating(|| book.expel(task, c, p)), Ok(()));
    assert_eq!(without_allocating(|| book.waive(task)), Ok(()));
    assert_eq!(without_allocating(|| book.unbind(installer, task)), Ok(()));
}

#[test]
fn all_seventeen_faces_reject_every_other_action_without_mutation() {
    use env::TaskId;
    use service::identity::*;
    let installer = TaskId::new(1);
    let task = TaskId::new(3);
    let mut book = book::IdentityBook::new(TaskId::new(2), installer).unwrap();
    let p = book.root();
    let subject = Subject::new(p, &[]).unwrap();
    book.bind(installer, task, Install::Authorized(subject)).unwrap();
    let c = book.found(task).unwrap();
    let actions = [
        Wire::Resolve(task), Wire::Matches(task, Selector::Exact(p)), Wire::Same(task, task),
        Wire::Sire(p), Wire::Heir(p, p), Wire::Amid(p, c), Wire::Members(c, None),
        Wire::Memberships(p, None), Wire::Adopt(subject), Wire::Waive, Wire::Restrict(subject),
        Wire::Derive(p), Wire::Found, Wire::Admit(c, p), Wire::Expel(c, p),
        Wire::Bind(task, Install::Authorized(subject)), Wire::Unbind(task),
    ];
    for face in Grant::ALL {
        for wire in actions {
            if Grant::for_wire(&wire) == face { continue; }
            let before = book.resolve(task);
            assert_eq!(answer::answer(&mut book, installer, face, Some(wire)),
                Reply::Fail(Fail::Denied));
            assert_eq!(book.resolve(task), before);
            assert_eq!(book.amid(p, c), Ok(false));
        }
    }
}

#[test]
fn adopt_keeps_the_origin_and_restrict_moves_it() {
    use env::TaskId;
    use service::identity::*;
    let installer = TaskId::new(1);
    let task = TaskId::new(3);
    let mut book = book::IdentityBook::new(TaskId::new(2), installer).unwrap();
    let p = book.derive(installer, book.root()).unwrap();
    book.bind(installer, task, Install::Authorized(Subject::new(p, &[]).unwrap())).unwrap();
    let q = book.derive(task, p).unwrap();
    let narrowed = Subject::new(q, &[]).unwrap();
    assert_eq!(answer::answer(&mut book, task, Grant::Adopt, Some(Wire::Adopt(narrowed))),
        Reply::Unit);
    assert_eq!(book.resolve(task).unwrap().origin.principal, p);
    assert_eq!(book.resolve(task).unwrap().current.principal, q);
    assert_eq!(answer::answer(&mut book, task, Grant::Restrict, Some(Wire::Restrict(narrowed))),
        Reply::Unit);
    assert_eq!(book.resolve(task).unwrap().origin.principal, q);
}
