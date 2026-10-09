use crate::gate::{self, AnyPie};
use crate::work::unit::task::Task;
use env::{Mark, Permission, PieFail, PieToken};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

thread_local! { static FAIL: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let fail = FAIL
            .try_with(|count| match count.get() {
                Some(0) => {
                    count.set(None);
                    true
                }
                Some(n) => {
                    count.set(Some(n - 1));
                    false
                }
                None => false,
            })
            .unwrap_or(false);
        if fail {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(ptr, layout);
        }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn root(task: &Arc<Task>, permission: Permission) -> PieToken {
    let pie = gate::boxed(gate::new_pie::<gate::Nole>(
        crate::work::mail::nole::NoleMeta::new(task.ident.id),
        Mark::of("gate-test"),
        permission,
        None,
    )).expect("pie allocation");
    let token = pie.token();
    gate::insert(task, pie).unwrap();
    token
}
fn rights() -> Permission {
    Permission::FETCH | Permission::STORE | Permission::VEST
}
fn grant(task: &Arc<Task>, token: PieToken, target: &Arc<Task>) -> Result<PieToken, PieFail> {
    gate::accord(task, token, &Arc::downgrade(target), rights(), Mark::NONE).map(PieToken::mint)
}
fn release(task: &Arc<Task>, token: PieToken) -> Result<usize, PieFail> {
    for _ in 0..1000 {
        match gate::release(task, token) {
            Err(PieFail::Busy) => thread::yield_now(),
            result => return result,
        }
    }
    Err(PieFail::Busy)
}
fn check(task: &Arc<Task>) {
    let edges = task.heirs.lock();
    for (parent, child, token) in edges.iter() {
        assert!(gate::locate(task, *parent).is_some());
        let child = child.upgrade().expect("live child");
        let pie = gate::locate(&child, *token).expect("indexed child");
        assert_eq!(pie.sire(), Some(*parent));
        assert!(pie.lord().ptr_eq(&Arc::downgrade(task)));
    }
    assert!(
        edges
            .windows(2)
            .all(|pair| pair[0].0.get() <= pair[1].0.get())
    );
}

#[test]
fn source_is_found_without_registering_any_task() {
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let child = grant(&a, token, &b).unwrap();
    assert_eq!(gate::vestor(&b, child), Some(a.ident.id));
    assert_eq!(gate::vestor(&a, token), None);
    check(&a);
    check(&b);
}
#[test]
fn self_transfer_and_release_do_not_lock_twice() {
    let task = Task::new();
    let token = root(&task, rights());
    let child = grant(&task, token, &task).unwrap();
    assert_eq!(gate::vestor(&task, child), Some(task.ident.id));
    assert_eq!(release(&task, token), Ok(2));
    assert!(task.pies.lock().is_empty());
    assert!(task.heirs.lock().is_empty());
}
#[test]
fn siblings_and_descendants_are_removed_but_unrelated_roots_survive() {
    let a = Task::new();
    let b = Task::new();
    let c = Task::new();
    let d = Task::new();
    let token = root(&a, rights());
    let other = root(&b, rights());
    let ab = grant(&a, token, &b).unwrap();
    let ac = grant(&a, token, &c).unwrap();
    let bd = grant(&b, ab, &d).unwrap();
    assert_eq!(release(&a, token), Ok(4));
    for (task, token) in [(&b, ab), (&c, ac), (&d, bd)] {
        assert!(gate::locate(task, token).is_none());
    }
    assert!(gate::locate(&b, other).is_some());
    for task in [&a, &b, &c, &d] {
        check(task);
    }
}
#[test]
fn unrelated_task_cannot_revoke_a_child() {
    let a = Task::new();
    let b = Task::new();
    let other = Task::new();
    let token = root(&a, rights());
    let child = grant(&a, token, &b).unwrap();
    assert_eq!(
        gate::revoke(&other, &Arc::downgrade(&b), child),
        Err(PieFail::Denied)
    );
    assert!(gate::locate(&b, child).is_some());
    check(&a);
}
#[test]
fn forget_updates_parent_task_and_transfer_records() {
    let a = Task::new();
    let b = Task::new();
    let c = Task::new();
    let d = Task::new();
    let token = root(&a, rights());
    let ab = grant(&a, token, &b).unwrap();
    let bc = grant(&b, ab, &c).unwrap();
    let cd = grant(&c, bc, &d).unwrap();
    gate::forget(&b, ab).unwrap();
    assert_eq!(gate::vestor(&c, bc), Some(a.ident.id));
    assert_eq!(gate::vestor(&d, cd), Some(c.ident.id));
    assert!(gate::locate(&b, ab).is_none());
    for task in [&a, &b, &c, &d] {
        check(task);
    }
    assert_eq!(release(&a, token), Ok(3));
    assert!(c.pies.lock().is_empty());
    assert!(d.pies.lock().is_empty());
}
#[test]
fn forget_handles_a_child_in_the_same_task() {
    let task = Task::new();
    let token = root(&task, rights());
    let child = grant(&task, token, &task).unwrap();
    let grandchild = grant(&task, child, &task).unwrap();
    gate::forget(&task, child).unwrap();
    assert_eq!(gate::locate(&task, grandchild).unwrap().sire(), Some(token));
    check(&task);
    assert_eq!(release(&task, token), Ok(2));
}
#[test]
fn revocation_restores_exclusive_parent_and_stale_clear_cannot_erase_new_heir() {
    let a = Task::new();
    let b = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&a, permission);
    let first = PieToken::mint(
        gate::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE).unwrap(),
    );
    let old = gate::locate(&a, token).unwrap().heir().copied().unwrap();
    assert_eq!(
        gate::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE),
        Err(PieFail::HandedOver)
    );
    gate::revoke(&a, &Arc::downgrade(&b), first).unwrap();
    let next = PieToken::mint(
        gate::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE).unwrap(),
    );
    assert!(!gate::clear_heir(&a, token, old));
    assert_eq!(gate::locate(&a, token).unwrap().heir().unwrap().token, next);
    check(&a);
}
#[test]
fn changed_version_prevents_any_commit() {
    let a = Task::new();
    let token = root(&a, rights());
    let version = a.version.load(Ordering::Relaxed);
    let tasks = vec![(a.clone(), version)];
    root(&a, rights());
    let mut called = false;
    assert_eq!(
        gate::with_tasks(&tasks, || {
            called = true;
        }),
        Err(PieFail::Busy)
    );
    assert!(!called);
    assert!(gate::locate(&a, token).is_some());
}
#[test]
fn allocation_failure_leaves_both_transfer_records_unchanged() {
    for successful_allocations in [0, 1] {
        let a = Task::new();
        let b = Task::new();
        let permission = rights() | Permission::ONLY;
        let token = root(&a, permission);
        FAIL.with(|count| count.set(Some(successful_allocations)));
        let result = gate::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE);
        FAIL.with(|count| count.set(None));
        assert_eq!(result, Err(PieFail::OoM));
        assert!(a.heirs.lock().is_empty());
        assert!(b.pies.lock().is_empty());
        assert!(gate::locate(&a, token).unwrap().heir().is_none());
    }
}
#[test]
fn allocation_failure_during_revoke_preserves_authority_and_records() {
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let child = grant(&a, token, &b).unwrap();
    FAIL.with(|count| count.set(Some(0)));
    let result = gate::revoke(&a, &Arc::downgrade(&b), child);
    FAIL.with(|count| count.set(None));
    assert_eq!(result, Err(PieFail::OoM));
    assert!(gate::locate(&b, child).is_some());
    check(&a);
}
#[test]
fn independent_transfer_progresses_while_another_task_gate_is_held() {
    let unrelated = Task::new();
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let guard = unrelated.gate.lock();
    let (send, receive) = std::sync::mpsc::channel();
    let raw = token.get();
    let worker = thread::spawn(move || {
        send.send(grant(&a, PieToken::mint(raw), &b).map(|token| token.get()))
            .unwrap()
    });
    let result = receive.recv_timeout(Duration::from_secs(3));
    drop(guard);
    worker.join().unwrap();
    assert!(result.unwrap().is_ok());
}
#[test]
fn opposite_transfers_complete_without_deadlock() {
    let a = Task::new();
    let b = Task::new();
    let ab = root(&a, rights());
    let ba = root(&b, rights());
    thread::scope(|scope| {
        for (source, token, target) in [(&a, ab, &b), (&b, ba, &a)] {
            let raw = token.get();
            scope.spawn(move || {
                for _ in 0..200 {
                    let token = PieToken::mint(raw);
                    let child = grant(source, token, target).unwrap();
                    assert_eq!(release(target, child), Ok(1));
                }
            });
        }
    });
    check(&a);
    check(&b);
    assert!(a.heirs.lock().is_empty());
    assert!(b.heirs.lock().is_empty());
}
#[test]
fn concurrent_transfer_and_revoke_cannot_leave_a_descendant() {
    for _ in 0..200 {
        let a = Task::new();
        let b = Task::new();
        let c = Task::new();
        let token = root(&a, rights());
        let ab = grant(&a, token, &b).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        thread::scope(|scope| {
            let barrier2 = barrier.clone();
            let b2 = b.clone();
            let c2 = c.clone();
            let raw = ab.get();
            scope.spawn(move || {
                barrier2.wait();
                match grant(&b2, PieToken::mint(raw), &c2) {
                    Ok(_) | Err(PieFail::Denied) => (),
                    result => panic!("{result:?}"),
                }
            });
            barrier.wait();
            assert!(release(&b, ab).is_ok());
        });
        assert!(b.pies.lock().is_empty());
        assert!(c.pies.lock().is_empty());
        check(&a);
        check(&b);
        check(&c);
    }
}
#[test]
fn concurrent_forget_and_revoke_cannot_leave_a_reparented_child() {
    for _ in 0..200 {
        let a = Task::new();
        let b = Task::new();
        let c = Task::new();
        let token = root(&a, rights());
        let ab = grant(&a, token, &b).unwrap();
        grant(&b, ab, &c).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        thread::scope(|scope| {
            let barrier2 = barrier.clone();
            let b2 = b.clone();
            let raw = ab.get();
            scope.spawn(move || {
                barrier2.wait();
                match gate::forget(&b2, PieToken::mint(raw)) {
                    Ok(()) | Err(PieFail::Denied | PieFail::Busy) => (),
                    result => panic!("{result:?}"),
                }
            });
            barrier.wait();
            assert!(release(&a, token).is_ok());
        });
        for task in [&a, &b, &c] {
            assert!(task.pies.lock().is_empty());
            assert!(task.heirs.lock().is_empty());
        }
    }
}
#[test]
fn exit_closes_delivery_and_removes_incoming_and_outgoing_transfers() {
    for _ in 0..100 {
        let a = Task::new();
        let b = Task::new();
        let c = Task::new();
        let token = root(&a, rights());
        let ab = grant(&a, token, &b).unwrap();
        grant(&b, ab, &c).unwrap();
        let own = root(&b, rights());
        let meta = gate::locate(&b, own).unwrap().nole().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        thread::scope(|scope| {
            let barrier2 = barrier.clone();
            let a2 = a.clone();
            let b2 = b.clone();
            let raw = token.get();
            scope.spawn(move || {
                barrier2.wait();
                let _ = grant(&a2, PieToken::mint(raw), &b2);
            });
            barrier.wait();
            gate::doom(&b);
        });
        assert!(!meta.alive());
        assert!(b.pies.lock().is_empty());
        assert!(c.pies.lock().is_empty());
        assert_eq!(grant(&a, token, &b), Err(PieFail::Dead));
        check(&a);
        assert!(a.heirs.lock().is_empty());
    }
}
#[test]
fn large_revoke_involves_only_actual_descendants() {
    let a = Task::new();
    let token = root(&a, rights());
    let children: Vec<_> = (0..64).map(|_| Task::new()).collect();
    for child in &children {
        grant(&a, token, child).unwrap();
    }
    assert_eq!(release(&a, token), Ok(65));
    assert!(a.heirs.lock().is_empty());
    assert!(children.iter().all(|child| child.pies.lock().is_empty()));
}

#[test]
fn clear_heir_rechecks_that_the_recipient_still_has_the_capability() {
    let a = Task::new();
    let b = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&a, permission);
    gate::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE).unwrap();
    let observed = gate::locate(&a, token).unwrap().heir().copied().unwrap();
    assert!(!gate::clear_heir(&a, token, observed));
    assert_eq!(gate::locate(&a, token).unwrap().heir(), Some(&observed));
    check(&a);
}

#[test]
fn allocation_failure_during_forget_keeps_every_child_attached_to_its_parent() {
    for successful_allocations in 0..4 {
        let a = Task::new();
        let b = Task::new();
        let token = root(&a, rights());
        let ab = grant(&a, token, &b).unwrap();
        let children: Vec<_> = (0..8).map(|_| Task::new()).collect();
        let tokens: Vec<_> = children
            .iter()
            .map(|child| grant(&b, ab, child).unwrap())
            .collect();
        FAIL.with(|count| count.set(Some(successful_allocations)));
        let result = gate::forget(&b, ab);
        FAIL.with(|count| count.set(None));
        assert_eq!(result, Err(PieFail::OoM));
        assert!(gate::locate(&b, ab).is_some());
        for (child, token) in children.iter().zip(tokens) {
            assert_eq!(gate::vestor(child, token), Some(b.ident.id));
        }
        check(&a);
        check(&b);
    }
}

#[test]
fn exit_removes_borrowed_memory_even_while_an_operation_is_in_progress() {
    let a = Task::new();
    let b = Task::new();
    let c = Task::new();
    let meta = crate::work::mail::pole::PoleMeta::new(a.ident.id);
    let pie = gate::boxed(gate::try_new_pie::<gate::Pole>(meta.clone(), Mark::NONE, rights(), None).unwrap()).expect("pie allocation");
    let token = pie.token();
    gate::insert(&a, pie).unwrap();
    let ab = grant(&a, token, &b).unwrap();
    grant(&b, ab, &c).unwrap();
    let _operation = meta.backing().operation().unwrap();
    gate::doom(&b);
    assert!(b.pies.lock().is_empty());
    assert!(c.pies.lock().is_empty());
    assert!(a.heirs.lock().is_empty());
}

#[test]
fn unrelated_version_changes_do_not_block_release_or_revoke() {
    for revoke in [false, true] {
        let a = Task::new();
        let b = Task::new();
        let token = root(&a, rights());
        let unrelated = root(&a, rights());
        let ab = grant(&a, token, &b).unwrap();
        let children: Vec<_> = (0..64).map(|_| Task::new()).collect();
        for child in &children { grant(&b, ab, child).unwrap(); }
        let a2 = a.clone();
        let raw = unrelated.get();
        crate::lock::before_locking(move || {
            gate::reduce(&a2, PieToken::mint(raw), Permission::FETCH).unwrap();
        });
        let result = if revoke { gate::revoke(&a, &Arc::downgrade(&b), ab) }
            else { gate::release(&b, ab) };
        assert_eq!(result, Ok(65));
        assert!(a.heirs.lock().is_empty());
        assert!(b.pies.lock().is_empty());
        assert!(children.iter().all(|child| child.pies.lock().is_empty()));
    }
}

#[test]
fn new_descendant_between_collection_and_locking_requires_recollection() {
    let a = Task::new();
    let b = Task::new();
    let c = Task::new();
    let token = root(&a, rights());
    let ab = grant(&a, token, &b).unwrap();
    let b2 = b.clone();
    let c2 = c.clone();
    let raw = ab.get();
    crate::lock::before_locking(move || { grant(&b2, PieToken::mint(raw), &c2).unwrap(); });
    assert_eq!(gate::release(&b, ab), Ok(2));
    assert!(b.pies.lock().is_empty());
    assert!(c.pies.lock().is_empty());
    check(&a);
}

#[test]
fn reparenting_to_an_unlocked_task_requires_recollection() {
    let z = Task::new();
    let a = Task::new();
    let b = Task::new();
    let token = root(&z, rights());
    let za = grant(&z, token, &a).unwrap();
    let ab = grant(&a, za, &b).unwrap();
    let a2 = a.clone();
    let raw = za.get();
    crate::lock::before_locking(move || { gate::forget(&a2, PieToken::mint(raw)).unwrap(); });
    assert_eq!(gate::release(&b, ab), Ok(1));
    assert!(z.heirs.lock().is_empty());
    assert!(a.pies.lock().is_empty());
    assert!(b.pies.lock().is_empty());
    assert!(gate::locate(&z, token).is_some());
}

#[test]
fn discard_appends_to_mail_abi_without_changing_existing_calls() {
    use env::{MailCall, HoleDir, VirtAddr, Wait};
    let token = PieToken::mint(42);
    for (index, call) in [
        MailCall::Push { token, msg: VirtAddr::new(0), len: 1 },
        MailCall::Pull { token, buf: VirtAddr::new(0), max: 64 },
        MailCall::Wait { token, dir: HoleDir::Pull, millis: Wait::POLL },
        MailCall::Hush { token },
        MailCall::Ring { token },
        MailCall::Peek { token },
        MailCall::Discard { token, max: 64 },
    ].into_iter().enumerate() {
        assert_eq!(call.slot(), (5usize << 32) | index);
        assert!(matches!(env::EnvCall::from_wire(call.slot(), &call.pack()), Ok(env::EnvCall::Mail(decoded)) if decoded == call));
    }
}

#[test]
fn locating_and_accessing_snapshots_do_not_allocate() {
    let task = Task::new();
    let token = root(&task, rights());
    FAIL.with(|count| count.set(Some(0)));
    let first = gate::locate(&task, token).unwrap();
    let second = gate::locate(&task, token).unwrap();
    let meta = first.nole().unwrap();
    let same = first.same(&second);
    let unspent = FAIL.with(|count| count.replace(None));
    assert_eq!(unspent, Some(0));
    assert!(same);
    assert!(meta.alive());
    assert!(first.hole().is_none());
    assert!(first.pole().is_none());
    assert!(first.tole().is_none());
}

#[test]
fn erased_resources_keep_their_type_identity_across_grants() {
    use crate::work::mail::{HoleMeta, NoleMeta, PoleMeta, ToleMeta};
    let a = Task::new();
    let b = Task::new();
    let entries = [
        gate::boxed(gate::new_pie::<gate::Hole>(HoleMeta::new(a.ident.id), Mark::NONE, rights(), None)).unwrap(),
        gate::boxed(gate::new_pie::<gate::Pole>(PoleMeta::new(a.ident.id), Mark::NONE, rights(), None)).unwrap(),
        gate::boxed(gate::new_pie::<gate::Nole>(NoleMeta::new(a.ident.id), Mark::NONE, rights(), None)).unwrap(),
        gate::boxed(gate::new_pie::<gate::Tole>(ToleMeta::new(a.ident.id), Mark::NONE, rights(), None)).unwrap(),
    ];
    let mut previous = None;
    for entry in entries {
        let kind = entry.kind();
        let token = entry.token();
        gate::insert(&a, entry).unwrap();
        let child = grant(&a, token, &b).unwrap();
        let source = gate::locate(&a, token).unwrap();
        let granted = gate::locate(&b, child).unwrap();
        assert_eq!(source.kind(), kind);
        assert_eq!(granted.kind(), kind);
        assert!(source.same(&granted));
        assert_eq!(source.owner(), Some(a.ident.id));
        assert_eq!(usize::from(source.hole().is_some()) + usize::from(source.pole().is_some())
            + usize::from(source.nole().is_some()) + usize::from(source.tole().is_some()), 1);
        if let Some(previous) = &previous { assert!(!source.same(previous)); }
        previous = Some(source);
    }
}

#[test]
fn granted_references_keep_independent_permissions_and_relations() {
    let a = Task::new();
    let b = Task::new();
    let c = Task::new();
    let token = root(&a, rights());
    let badge = Mark::of("recipient");
    let child = PieToken::mint(gate::accord(&a, token, &Arc::downgrade(&b), rights(), badge).unwrap());
    let descendant = grant(&b, child, &c).unwrap();
    gate::reduce(&b, child, Permission::FETCH).unwrap();
    assert_eq!(gate::locate(&a, token).unwrap().permission(), rights());
    assert_eq!(gate::locate(&b, child).unwrap().permission(), Permission::FETCH);
    assert_eq!(gate::locate(&c, descendant).unwrap().permission(), rights());
    assert_eq!(gate::locate(&b, child).unwrap().mark(), badge);
    assert_eq!(gate::locate(&a, token).unwrap().mark(), Mark::of("gate-test"));
    assert_eq!(gate::vestor(&b, child), Some(a.ident.id));
    gate::forget(&b, child).unwrap();
    assert_eq!(gate::vestor(&c, descendant), Some(a.ident.id));
    assert!(gate::locate(&a, token).unwrap().sire().is_none());
    assert_eq!(gate::locate(&c, descendant).unwrap().sire(), Some(token));
    check(&a);
    check(&b);
    check(&c);
}

#[test]
fn memory_snapshots_observe_permission_reduction_and_revocation() {
    let a = Task::new();
    let b = Task::new();
    let meta = crate::work::mail::PoleMeta::new(a.ident.id);
    let entry = gate::boxed(gate::new_pie::<gate::Pole>(meta.clone(), Mark::NONE, rights(), None)).unwrap();
    let token = entry.token();
    gate::insert(&a, entry).unwrap();
    let child = grant(&a, token, &b).unwrap();
    let retained = gate::locate(&b, child).unwrap();
    gate::reduce(&b, child, Permission::FETCH).unwrap();
    assert_eq!(retained.permission(), Permission::FETCH);
    assert_eq!(gate::locate(&a, token).unwrap().permission(), rights());
    gate::revoke(&a, &Arc::downgrade(&b), child).unwrap();
    assert!(gate::locate(&b, child).is_none());
    assert!(retained.permission().is_empty());
    assert!(retained.alive());
    assert!(Arc::ptr_eq(&retained.pole().unwrap(), &meta));
}

#[test]
fn box_allocation_failure_does_not_publish_an_exclusive_grant() {
    let a = Task::new();
    let b = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&a, permission);
    let before_a = a.version.load(Ordering::Relaxed);
    let before_b = b.version.load(Ordering::Relaxed);
    FAIL.with(|count| count.set(Some(0)));
    let result = gate::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE);
    FAIL.with(|count| count.set(None));
    assert_eq!(result, Err(PieFail::OoM));
    assert!(gate::locate(&a, token).unwrap().heir().is_none());
    assert!(a.heirs.lock().is_empty());
    assert!(b.pies.lock().is_empty());
    assert_eq!(a.version.load(Ordering::Relaxed), before_a);
    assert_eq!(b.version.load(Ordering::Relaxed), before_b);
    check(&a);
    check(&b);
}
