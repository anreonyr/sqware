use crate::pend::{self, Pie};
use crate::core::work::mail::Mail;
use crate::core::work::unit::task::Task;
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
    let pie = pend::Pie::try_new(
        crate::core::work::mail::Mail::Nole(crate::core::work::mail::nole::NoleMeta::new(task.ident.id)),
        Mark::of("pend-test"),
        permission,
        None,
    )
    .expect("pie allocation");
    let token = pie.token();
    pend::insert(task, pie).unwrap();
    token
}
fn pie(meta: crate::core::work::mail::Mail, permission: Permission) -> Pie {
    pend::Pie::try_new(meta, Mark::NONE, permission, None).expect("pie allocation")
}
fn rights() -> Permission {
    Permission::FETCH | Permission::STORE | Permission::VEST
}

#[test]
fn availability_rechecks_delivery_without_clearing_cached_relations() {
    let owner = Task::new();
    let recipient = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&owner, permission);
    let child = pend::accord(
        &owner, token, &Arc::downgrade(&recipient), permission, Mark::NONE,
    ).map(PieToken::mint).unwrap();
    let snapshot = pend::locate(&owner, token).unwrap();
    let version = owner.pend.version.load(Ordering::Relaxed);
    assert!(snapshot.heir().is_some());
    assert_eq!(pend::available(&snapshot), Err(env::PieFail::HandedOver));
    assert_eq!(owner.pend.version.load(Ordering::Relaxed), version);

    assert_eq!(release(&recipient, child), Ok(1));
    assert_eq!(pend::available(&snapshot), Ok(()));
    assert!(snapshot.heir().is_some());
}

#[test]
fn availability_observes_staging_without_a_second_authority_record() {
    let owner = Task::new();
    let meta = crate::core::work::mail::PoleMeta::new(owner.ident.id);
    let reference = pie(Mail::Pole(meta.clone()), rights());
    let token = reference.token();
    pend::insert(&owner, reference).unwrap();
    let snapshot = pend::locate(&owner, token).unwrap();
    assert_eq!(pend::available(&snapshot), Ok(()));
    meta.stage_for(9);
    assert_eq!(pend::available(&snapshot), Err(env::PieFail::HandedOver));
    meta.stage_for(0);
    assert_eq!(pend::available(&snapshot), Ok(()));
}

fn grant(task: &Arc<Task>, token: PieToken, target: &Arc<Task>) -> Result<PieToken, PieFail> {
    pend::accord(task, token, &Arc::downgrade(target), rights(), Mark::NONE).map(PieToken::mint)
}
fn release(task: &Arc<Task>, token: PieToken) -> Result<usize, PieFail> {
    for _ in 0..1000 {
        match pend::release(task, token) {
            Err(PieFail::Busy) => thread::yield_now(),
            result => return result,
        }
    }
    Err(PieFail::Busy)
}
fn check(task: &Arc<Task>) {
    let edges = task.pend.heirs.lock();
    for (parent, child, token) in edges.iter() {
        assert!(pend::locate(task, *parent).is_some());
        let child = child.upgrade().expect("live child");
        let pie = pend::locate(&child, *token).expect("indexed child");
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
    assert_eq!(pend::vestor(&b, child), Some(a.ident.id));
    assert_eq!(pend::vestor(&a, token), None);
    check(&a);
    check(&b);
}
#[test]
fn self_transfer_and_release_do_not_lock_twice() {
    let task = Task::new();
    let token = root(&task, rights());
    let child = grant(&task, token, &task).unwrap();
    assert_eq!(pend::vestor(&task, child), Some(task.ident.id));
    assert_eq!(release(&task, token), Ok(2));
    assert!(task.pend.pies.lock().is_empty());
    assert!(task.pend.heirs.lock().is_empty());
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
        assert!(pend::locate(task, token).is_none());
    }
    assert!(pend::locate(&b, other).is_some());
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
        pend::revoke(&other, &Arc::downgrade(&b), child),
        Err(PieFail::Denied)
    );
    assert!(pend::locate(&b, child).is_some());
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
    pend::forget(&b, ab).unwrap();
    assert_eq!(pend::vestor(&c, bc), Some(a.ident.id));
    assert_eq!(pend::vestor(&d, cd), Some(c.ident.id));
    assert!(pend::locate(&b, ab).is_none());
    for task in [&a, &b, &c, &d] {
        check(task);
    }
    assert_eq!(release(&a, token), Ok(3));
    assert!(c.pend.pies.lock().is_empty());
    assert!(d.pend.pies.lock().is_empty());
}
#[test]
fn forget_handles_a_child_in_the_same_task() {
    let task = Task::new();
    let token = root(&task, rights());
    let child = grant(&task, token, &task).unwrap();
    let grandchild = grant(&task, child, &task).unwrap();
    pend::forget(&task, child).unwrap();
    assert_eq!(pend::locate(&task, grandchild).unwrap().sire(), Some(token));
    check(&task);
    assert_eq!(release(&task, token), Ok(2));
}
#[test]
fn forget_releases_sealed_references_without_reviving_descendants() {
    let owner = Task::new();
    let holder = Task::new();
    let recipient = Task::new();
    let token = root(&owner, rights());
    let middle = grant(&owner, token, &holder).unwrap();
    let leaf = grant(&holder, middle, &recipient).unwrap();
    let before = pend::locate(&recipient, leaf).unwrap();
    pend::reduce(&owner, token, Permission::FETCH).unwrap();
    assert_eq!(pend::forget(&holder, middle), Err(PieFail::Denied));

    pend::locate(&owner, token).unwrap().mail().seal();
    assert_eq!(pend::forget(&holder, middle), Ok(()));
    assert!(pend::locate(&holder, middle).is_none());
    let after = pend::locate(&recipient, leaf).unwrap();
    assert_eq!(after.sire(), Some(token));
    assert_eq!(after.permission(), before.permission());
    assert!(Arc::ptr_eq(after.grant(), before.grant()));
    assert!(
        !after.alive(),
        "releasing a dead reference must not restore authority"
    );
    for task in [&owner, &holder, &recipient] {
        check(task);
    }
    assert_eq!(release(&owner, token), Ok(2));
}
#[test]
fn revocation_restores_exclusive_parent_and_stale_clear_cannot_erase_new_heir() {
    let a = Task::new();
    let b = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&a, permission);
    let first = PieToken::mint(
        pend::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE).unwrap(),
    );
    let old = pend::locate(&a, token).unwrap().heir().copied().unwrap();
    assert_eq!(
        pend::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE),
        Err(PieFail::HandedOver)
    );
    pend::revoke(&a, &Arc::downgrade(&b), first).unwrap();
    let next = PieToken::mint(
        pend::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE).unwrap(),
    );
    assert!(!pend::clear_heir(&a, token, old));
    assert_eq!(pend::locate(&a, token).unwrap().heir().unwrap().token, next);
    check(&a);
}
#[test]
fn changed_version_prevents_any_commit() {
    let a = Task::new();
    let token = root(&a, rights());
    let version = a.pend.version.load(Ordering::Relaxed);
    let tasks = vec![(a.clone(), version)];
    root(&a, rights());
    let mut called = false;
    assert_eq!(
        pend::with_tasks(&tasks, || {
            called = true;
        }),
        Err(PieFail::Busy)
    );
    assert!(!called);
    assert!(pend::locate(&a, token).is_some());
}
#[test]
fn allocation_failure_leaves_both_transfer_records_unchanged() {
    for successful_allocations in [0, 1] {
        let a = Task::new();
        let b = Task::new();
        let permission = rights() | Permission::ONLY;
        let token = root(&a, permission);
        FAIL.with(|count| count.set(Some(successful_allocations)));
        let result = pend::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE);
        FAIL.with(|count| count.set(None));
        assert_eq!(result, Err(PieFail::OoM));
        assert!(a.pend.heirs.lock().is_empty());
        assert!(b.pend.pies.lock().is_empty());
        assert!(pend::locate(&a, token).unwrap().heir().is_none());
    }
}
#[test]
fn allocation_failure_during_revoke_preserves_authority_and_records() {
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let child = grant(&a, token, &b).unwrap();
    FAIL.with(|count| count.set(Some(0)));
    let result = pend::revoke(&a, &Arc::downgrade(&b), child);
    FAIL.with(|count| count.set(None));
    assert_eq!(result, Err(PieFail::OoM));
    assert!(pend::locate(&b, child).is_some());
    check(&a);
}
#[test]
fn independent_transfer_progresses_while_another_task_pend_is_held() {
    let unrelated = Task::new();
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let guard = unrelated.pend.lock();
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
    assert!(a.pend.heirs.lock().is_empty());
    assert!(b.pend.heirs.lock().is_empty());
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
        assert!(b.pend.pies.lock().is_empty());
        assert!(c.pend.pies.lock().is_empty());
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
                match pend::forget(&b2, PieToken::mint(raw)) {
                    Ok(()) | Err(PieFail::Denied | PieFail::Busy) => (),
                    result => panic!("{result:?}"),
                }
            });
            barrier.wait();
            assert!(release(&a, token).is_ok());
        });
        for task in [&a, &b, &c] {
            assert!(task.pend.pies.lock().is_empty());
            assert!(task.pend.heirs.lock().is_empty());
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
        let meta = pend::locate(&b, own).unwrap().mail().nole().unwrap();
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
            { let _commit = crate::core::work::commit(); b.state.lock().doom(None, env::ExitCause::Slay, 0); }
            pend::doom(&b);
        });
        assert!(!meta.alive());
        assert!(b.pend.pies.lock().is_empty());
        assert!(c.pend.pies.lock().is_empty());
        assert_eq!(grant(&a, token, &b), Err(PieFail::Dead));
        check(&a);
        assert!(a.pend.heirs.lock().is_empty());
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
    assert!(a.pend.heirs.lock().is_empty());
    assert!(children.iter().all(|child| child.pend.pies.lock().is_empty()));
}

#[test]
fn clear_heir_rechecks_that_the_recipient_still_has_the_capability() {
    let a = Task::new();
    let b = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&a, permission);
    pend::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE).unwrap();
    let observed = pend::locate(&a, token).unwrap().heir().copied().unwrap();
    assert!(!pend::clear_heir(&a, token, observed));
    assert_eq!(pend::locate(&a, token).unwrap().heir(), Some(&observed));
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
        let result = pend::forget(&b, ab);
        FAIL.with(|count| count.set(None));
        assert_eq!(result, Err(PieFail::OoM));
        assert!(pend::locate(&b, ab).is_some());
        for (child, token) in children.iter().zip(tokens) {
            assert_eq!(pend::vestor(child, token), Some(b.ident.id));
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
    let meta = crate::core::work::mail::pole::PoleMeta::new(a.ident.id);
    let pie = pend::Pie::try_new(
        crate::core::work::mail::Mail::Pole(meta.clone()),
        Mark::NONE,
        rights(),
        None,
    )
    .expect("pie allocation");
    let token = pie.token();
    pend::insert(&a, pie).unwrap();
    let ab = grant(&a, token, &b).unwrap();
    grant(&b, ab, &c).unwrap();
    let _operation = crate::core::work::mail::pole::PoleOperation::acquire(&meta).unwrap();
    { let _commit = crate::core::work::commit(); b.state.lock().doom(None, env::ExitCause::Slay, 0); }
    pend::doom(&b);
    assert!(b.pend.pies.lock().is_empty());
    assert!(c.pend.pies.lock().is_empty());
    assert!(a.pend.heirs.lock().is_empty());
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
        crate::machine::sync::before_locking(move || {
            pend::reduce(&a2, PieToken::mint(raw), Permission::FETCH).unwrap();
        });
        let result = if revoke { pend::revoke(&a, &Arc::downgrade(&b), ab) }
            else { pend::release(&b, ab) };
        assert_eq!(result, Ok(65));
        assert!(a.pend.heirs.lock().is_empty());
        assert!(b.pend.pies.lock().is_empty());
        assert!(children.iter().all(|child| child.pend.pies.lock().is_empty()));
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
    crate::machine::sync::before_locking(move || { grant(&b2, PieToken::mint(raw), &c2).unwrap(); });
    assert_eq!(pend::release(&b, ab), Ok(2));
    assert!(b.pend.pies.lock().is_empty());
    assert!(c.pend.pies.lock().is_empty());
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
    crate::machine::sync::before_locking(move || { pend::forget(&a2, PieToken::mint(raw)).unwrap(); });
    assert_eq!(pend::release(&b, ab), Ok(1));
    assert!(z.pend.heirs.lock().is_empty());
    assert!(a.pend.pies.lock().is_empty());
    assert!(b.pend.pies.lock().is_empty());
    assert!(pend::locate(&z, token).is_some());
}

#[test]
fn capability_and_resource_calls_roundtrip_with_explicit_numbers() {
    use env::{
        HoleCall, HoleCondition, Oversize, PieCall, ReleaseMode, TaskId, ToleCall, ToleEventSource,
        VirtAddr, Wait,
    };
    let token = PieToken::mint(42);
    let buf = VirtAddr::new(4096);
    let hole = [
        HoleCall::Unseal {
            mark: Mark::new(u64::MAX),
            limits: env::HoleLimits::default(),
        },
        HoleCall::Seal { token },
        HoleCall::Push {
            token,
            msg: buf,
            len: 1,
        },
        HoleCall::Pull {
            token,
            buf,
            max: 64,
            oversize: Oversize::Discard,
        },
        HoleCall::Peek { token },
        HoleCall::Wait {
            token,
            condition: HoleCondition::Empty,
            millis: Wait::POLL,
        },
        HoleCall::Ring { token },
        HoleCall::Hush { token },
    ];
    for (index, call) in hole.into_iter().enumerate() {
        assert_eq!(call.slot(), (5usize << 32) | index);
        assert!(
            matches!(env::EnvCall::from_wire(call.slot(), &call.pack()), Ok(env::EnvCall::Hole(decoded)) if decoded == call)
        );
    }
    let pie = [
        PieCall::Accord {
            src: token,
            dst: TaskId::new(2),
            subset: rights(),
            mark: Mark::NONE,
        },
        PieCall::Narrow {
            token,
            subset: rights(),
        },
        PieCall::Revoke {
            dst: TaskId::new(2),
            token,
        },
        PieCall::Release {
            token,
            mode: ReleaseMode::Keep,
        },
        PieCall::Inspect { token, buf },
        PieCall::Collect {
            after: token,
            buf,
            capacity: 8,
        },
        PieCall::Same { a: token, b: token },
    ];
    for (index, call) in pie.into_iter().enumerate() {
        assert_eq!(call.slot(), (7usize << 32) | (index + 4));
        assert!(
            matches!(env::EnvCall::from_wire(call.slot(), &call.pack()), Ok(env::EnvCall::Pie(decoded)) if decoded == call)
        );
    }
    let source = ToleEventSource::Hole {
        token,
        condition: HoleCondition::Pull,
    };
    for call in [
        ToleCall::Attach {
            tole: token,
            source,
        },
        ToleCall::Detach {
            tole: token,
            source,
        },
        ToleCall::Await {
            tole: token,
            millis: Wait::POLL,
        },
    ] {
        assert!(
            matches!(env::EnvCall::from_wire(call.slot(), &call.pack()), Ok(env::EnvCall::Tole(decoded)) if decoded == call)
        );
    }
    for index in 32..=40 {
        assert!(env::EnvCall::from_wire((5usize << 32) | index, &[0; 6]).is_err());
    }
    for index in 0..4 {
        for registers in [[0; 6], [usize::MAX; 6]] {
            assert_eq!(
                env::EnvCall::from_wire((7usize << 32) | index, &registers),
                Err(env::Decode::BadSlot)
            );
        }
    }
    for class in [3, 9, 13] {
        for index in 0..6 {
            assert!(env::EnvCall::from_wire((class << 32) | index, &[0; 6]).is_err());
        }
    }
    for (class, index) in [(5, 8), (10, 7), (11, 5), (12, 5)] {
        assert!(env::EnvCall::from_wire((class << 32) | index, &[0; 6]).is_err());
    }
}

#[test]
fn creation_and_conditions_reject_invalid_wire_values() {
    use env::{
        Decode, EnvCall, HoleCall, HoleCondition, HoleLimits, NoleCall, Oversize, PieCall,
        PoleCall, PoleCondition, ReleaseMode, ToleCall, VirtAddr, Wait,
    };
    for call in [
        EnvCall::Hole(HoleCall::Unseal {
            mark: Mark::new(u64::MAX),
            limits: HoleLimits::default(),
        }),
        EnvCall::Pole(PoleCall::Unseal {
            reference: env::FlatReference::from_bytes(&7usize.to_le_bytes()).unwrap(),
            shared: false,
        }),
        EnvCall::Nole(NoleCall::Unseal),
        EnvCall::Tole(ToleCall::Unseal { shared: true }),
    ] {
        let (slot, registers) = match call {
            EnvCall::Hole(call) => (call.slot(), call.pack()),
            EnvCall::Pole(call) => (call.slot(), call.pack()),
            EnvCall::Nole(call) => (call.slot(), call.pack()),
            EnvCall::Tole(call) => (call.slot(), call.pack()),
            _ => unreachable!(),
        };
        assert_eq!(EnvCall::from_wire(slot, &registers), Ok(call));
    }
    for (slot, registers) in [
        (5usize << 32, [0, 0, 4, 64, 0, 0]),
        (5usize << 32, [0, 64, 0, 64, 0, 0]),
        (5usize << 32, [0, 65, 4, 64, 0, 0]),
        (10usize << 32, [7, 2, 0, 0, 0, 0]),
        (10usize << 32, [usize::MAX, 0, 0, 0, 0, 0]),
        (12usize << 32, [2, 0, 0, 0, 0, 0]),
    ] {
        assert_eq!(EnvCall::from_wire(slot, &registers), Err(Decode::Invalid));
    }
    assert!(
        !HoleLimits {
            max_len: 0,
            max_messages: 1,
            max_bytes: 8
        }
        .valid()
    );
    let token = PieToken::mint(42);
    let call = HoleCall::Wait {
        token,
        condition: HoleCondition::Push,
        millis: Wait::POLL,
    };
    let mut regs = call.pack();
    regs[1] = 4;
    assert_eq!(
        HoleCall::from_wire(call.slot(), &regs),
        Err(Decode::Invalid)
    );
    let call = PoleCall::Wait {
        token,
        condition: PoleCondition::Signal(env::Bit::FIRST),
        millis: Wait::POLL,
    };
    let mut regs = call.pack();
    for invalid in [0, 1, 2, 3 + usize::BITS as usize, usize::MAX] {
        regs[1] = invalid;
        assert_eq!(
            PoleCall::from_wire(call.slot(), &regs),
            Err(Decode::Invalid)
        );
    }
    let call = HoleCall::Pull {
        token,
        buf: VirtAddr::new(4096),
        max: 8,
        oversize: Oversize::Keep,
    };
    let mut regs = call.pack();
    regs[3] = 2;
    assert_eq!(
        HoleCall::from_wire(call.slot(), &regs),
        Err(Decode::Invalid)
    );
    let call = PieCall::Release {
        token,
        mode: ReleaseMode::Keep,
    };
    let mut regs = call.pack();
    regs[1] = 2;
    assert_eq!(PieCall::from_wire(call.slot(), &regs), Err(Decode::Invalid));
}

#[test]
fn query_records_preserve_full_marks_permissions_and_closed_state() {
    use env::TaskId;
    let info = env::PieInfo {
        token: PieToken::mint(42),
        kind: env::PieKind::Pole,
        permission: rights() | Permission::ONLY,
        owner: TaskId::new(7),
        vestor: TaskId::new(8),
        mark: Mark::new(u64::MAX),
        alive: false,
    };
    assert_eq!(env::PieInfo::from_words(info.words()), Some(info));
    let mut words = info.words();
    words[1] = 256;
    assert_eq!(env::PieInfo::from_words(words), None);
    let mut words = info.words();
    words[2] |= 1usize << 40;
    assert_eq!(env::PieInfo::from_words(words), None);
    let mut words = info.words();
    words[6] = 2;
    assert_eq!(env::PieInfo::from_words(words), None);
}

#[test]
fn locating_and_accessing_snapshots_do_not_allocate() {
    let task = Task::new();
    let token = root(&task, rights());
    FAIL.with(|count| count.set(Some(0)));
    let first = pend::locate(&task, token).unwrap();
    let second = pend::locate(&task, token).unwrap();
    let meta = first.mail().nole().unwrap();
    let same = first.mail().same(second.mail());
    let unspent = FAIL.with(|count| count.replace(None));
    assert_eq!(unspent, Some(0));
    assert!(same);
    assert!(meta.alive());
    assert!(first.mail().hole().is_none());
    assert!(first.mail().pole().is_none());
    assert!(first.mail().tole().is_none());
}

#[test]
fn erased_resources_keep_their_type_identity_across_grants() {
    use crate::core::work::mail::{HoleMeta, Mail, NoleMeta, PoleMeta, ToleMeta};
    let a = Task::new();
    let b = Task::new();
    let entries = [
        pie(Mail::Hole(HoleMeta::new(a.ident.id)), rights()),
        pie(Mail::Pole(PoleMeta::new(a.ident.id)), rights()),
        pie(Mail::Nole(NoleMeta::new(a.ident.id)), rights()),
        pie(Mail::Tole(ToleMeta::new(a.ident.id)), rights()),
    ];
    let mut previous: Option<pend::PieSnapshot> = None;
    for entry in entries {
        let kind = entry.mail().kind();
        let token = entry.token();
        pend::insert(&a, entry).unwrap();
        let child = grant(&a, token, &b).unwrap();
        let source = pend::locate(&a, token).unwrap();
        let granted = pend::locate(&b, child).unwrap();
        assert_eq!(source.mail().kind(), kind);
        assert_eq!(granted.mail().kind(), kind);
        assert!(source.mail().same(granted.mail()));
        assert_eq!(source.owner(), Some(a.ident.id));
        assert_eq!(usize::from(source.mail().hole().is_some()) + usize::from(source.mail().pole().is_some())
            + usize::from(source.mail().nole().is_some()) + usize::from(source.mail().tole().is_some()), 1);
        if let Some(previous) = &previous { assert!(!source.mail().same(previous.mail())); }
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
    let child = PieToken::mint(pend::accord(&a, token, &Arc::downgrade(&b), rights(), badge).unwrap());
    let descendant = grant(&b, child, &c).unwrap();
    pend::reduce(&b, child, Permission::FETCH).unwrap();
    assert_eq!(pend::locate(&a, token).unwrap().permission(), rights());
    assert_eq!(pend::locate(&b, child).unwrap().permission(), Permission::FETCH);
    assert_eq!(pend::locate(&c, descendant).unwrap().permission(), rights());
    assert_eq!(pend::locate(&b, child).unwrap().mark(), badge);
    assert_eq!(pend::locate(&a, token).unwrap().mark(), Mark::of("pend-test"));
    assert_eq!(pend::vestor(&b, child), Some(a.ident.id));
    pend::forget(&b, child).unwrap();
    assert_eq!(pend::vestor(&c, descendant), Some(a.ident.id));
    assert!(pend::locate(&a, token).unwrap().sire().is_none());
    assert_eq!(pend::locate(&c, descendant).unwrap().sire(), Some(token));
    check(&a);
    check(&b);
    check(&c);
}

#[test]
fn memory_snapshots_observe_permission_reduction_and_revocation() {
    let a = Task::new();
    let b = Task::new();
    let meta = crate::core::work::mail::PoleMeta::new(a.ident.id);
    let entry = pie(Mail::Pole(meta.clone()), rights());
    let token = entry.token();
    pend::insert(&a, entry).unwrap();
    let child = grant(&a, token, &b).unwrap();
    let retained = pend::locate(&b, child).unwrap();
    let retained_copy = retained.clone();
    assert!(Arc::ptr_eq(retained.grant(), retained_copy.grant()));
    pend::reduce(&b, child, Permission::FETCH).unwrap();
    assert_eq!(retained.permission(), Permission::FETCH);
    assert_eq!(retained_copy.permission(), Permission::FETCH);
    assert_eq!(pend::locate(&a, token).unwrap().permission(), rights());
    pend::revoke(&a, &Arc::downgrade(&b), child).unwrap();
    assert!(pend::locate(&b, child).is_none());
    assert!(retained.permission().is_empty());
    assert!(retained_copy.permission().is_empty());
    assert!(!retained.alive());
    assert!(Arc::ptr_eq(&retained.mail().pole().unwrap(), &meta));
}

#[test]
fn box_allocation_failure_does_not_publish_an_exclusive_grant() {
    let a = Task::new();
    let b = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = root(&a, permission);
    let before_a = a.pend.version.load(Ordering::Relaxed);
    let before_b = b.pend.version.load(Ordering::Relaxed);
    FAIL.with(|count| count.set(Some(0)));
    let result = pend::accord(&a, token, &Arc::downgrade(&b), permission, Mark::NONE);
    FAIL.with(|count| count.set(None));
    assert_eq!(result, Err(PieFail::OoM));
    assert!(pend::locate(&a, token).unwrap().heir().is_none());
    assert!(a.pend.heirs.lock().is_empty());
    assert!(b.pend.pies.lock().is_empty());
    assert_eq!(a.pend.version.load(Ordering::Relaxed), before_a);
    assert_eq!(b.pend.version.load(Ordering::Relaxed), before_b);
    check(&a);
    check(&b);
}

/// 在任务表里落一枚无父的本地根，返它的号。
fn lone(task: &Arc<Task>, meta: Mail) -> PieToken {
    let entry = pie(meta, rights());
    let token = entry.token();
    pend::insert(task, entry).unwrap();
    token
}

/// 让下一次分配当场返回 null（且只失败这一次），跑一次 `pend::release`，
/// 再把失败钩回来：它仍是 `Some(0)` 就等于这一次释放一次也没分配。
fn release_without_allocating(task: &Arc<Task>, token: PieToken) -> Result<usize, PieFail> {
    FAIL.with(|count| count.set(Some(0)));
    let result = pend::release(task, token);
    let unspent = FAIL.with(|count| count.replace(None));
    assert_eq!(unspent, Some(0), "release allocated");
    result
}

#[test]
fn lone_roots_release_without_any_capacity() {
    use crate::core::work::mail::{HoleMeta, NoleMeta, PoleMeta};
    let task = Task::new();
    for meta in [
        Mail::Nole(NoleMeta::new(task.ident.id)),
        Mail::Hole(HoleMeta::new(task.ident.id)),
        Mail::Pole(PoleMeta::new(task.ident.id)),
    ] {
        let token = lone(&task, meta);
        assert_eq!(release_without_allocating(&task, token), Ok(1));
        assert!(pend::locate(&task, token).is_none());
        assert!(task.pend.pies.lock().is_empty());
        assert!(task.pend.heirs.lock().is_empty());
    }
}

#[test]
fn a_root_with_a_child_still_builds_the_node_table() {
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let child = grant(&a, token, &b).unwrap();
    FAIL.with(|count| count.set(Some(0)));
    let result = pend::release(&a, token);
    let spent = FAIL.with(|count| count.replace(None));
    assert_eq!(spent, None, "a root with a child still reserves its nodes");
    assert_eq!(result, Err(PieFail::OoM));
    assert!(pend::locate(&a, token).is_some());
    assert!(pend::locate(&b, child).is_some());
    check(&a);
    check(&b);
}

#[test]
fn a_child_still_builds_the_node_table() {
    let a = Task::new();
    let b = Task::new();
    let token = root(&a, rights());
    let child = grant(&a, token, &b).unwrap();
    FAIL.with(|count| count.set(Some(0)));
    let result = pend::release(&b, child);
    let spent = FAIL.with(|count| count.replace(None));
    assert_eq!(spent, None, "a child still reserves its nodes");
    assert_eq!(result, Err(PieFail::OoM));
    assert!(pend::locate(&b, child).is_some());
    assert!(pend::locate(&a, token).is_some());
    check(&a);
    check(&b);
}

#[test]
fn a_pole_with_a_recorded_map_still_builds_the_node_table() {
    use crate::core::work::mail::PoleMeta;
    let task = Task::new();
    let meta = PoleMeta::new(task.ident.id);
    let token = lone(&task, Mail::Pole(meta.clone()));
    meta.note_map();
    FAIL.with(|count| count.set(Some(0)));
    let result = pend::release(&task, token);
    let spent = FAIL.with(|count| count.replace(None));
    assert_eq!(spent, None, "a mapped Pole still reserves its nodes");
    assert_eq!(result, Err(PieFail::OoM));
    assert!(pend::locate(&task, token).is_some());
    assert_eq!(release(&task, token), Ok(1));
    assert!(task.pend.pies.lock().is_empty());
}

#[test]
fn a_pole_operation_lease_or_a_reservation_keeps_a_lone_root_busy() {
    use crate::core::work::mail::PoleMeta;
    use crate::core::work::mail::pole::PoleOperation;
    let task = Task::new();
    let meta = PoleMeta::new(task.ident.id);
    let token = lone(&task, Mail::Pole(meta.clone()));
    let lease = PoleOperation::acquire(&meta).unwrap();
    assert_eq!(pend::release(&task, token), Err(PieFail::Busy));
    assert!(pend::locate(&task, token).is_some());
    drop(lease);
    meta.stage_for(7);
    assert_eq!(pend::release(&task, token), Err(PieFail::Busy));
    assert!(pend::locate(&task, token).is_some());
    meta.stage_for(0);
    assert_eq!(pend::release(&task, token), Ok(1));
    assert!(task.pend.pies.lock().is_empty());
}

#[test]
fn an_in_flight_publication_keeps_a_lone_pole_root_busy() {
    use crate::core::memory::nest::Access;
    use crate::core::work::mail::PoleMeta;
    use crate::memory::address::PageCount;
    use crate::memory::flat::FlatRange;
    use crate::pend::SourceAuthorization;
    let task = Task::new();
    let meta = PoleMeta::new(task.ident.id);
    let source = SourceAuthorization::new(
        FlatRange::new(PageCount::new(1).unwrap()),
        Access::READ,
        task.ident.id,
    )
    .unwrap();
    meta.attach_source(&source);
    let token = lone(&task, Mail::Pole(meta.clone()));
    source.enter_publish(3, None, Access::READ).unwrap();
    assert_eq!(pend::release(&task, token), Err(PieFail::Busy));
    assert!(pend::locate(&task, token).is_some());
    source.leave_publish(3);
    assert_eq!(pend::release(&task, token), Ok(1));
    assert!(task.pend.pies.lock().is_empty());
}

#[test]
fn a_lone_root_retries_transient_contention_without_allocating() {
    use crate::core::work::mail::PoleMeta;
    let task = Task::new();
    let meta = PoleMeta::new(task.ident.id);
    let token = lone(&task, Mail::Pole(meta.clone()));
    meta.control.busy_for(1);
    assert_eq!(release_without_allocating(&task, token), Ok(1));
    assert!(pend::locate(&task, token).is_none());
}

#[test]
fn a_lone_root_preserves_the_bounded_contention_budget() {
    use crate::core::work::mail::PoleMeta;
    let task = Task::new();
    let meta = PoleMeta::new(task.ident.id);
    let token = lone(&task, Mail::Pole(meta.clone()));
    meta.control.busy_for(crate::pend::RETRIES + 1);
    assert_eq!(release_without_allocating(&task, token), Err(PieFail::Busy));
    assert_eq!(meta.control.busy_remaining(), 1);
    assert!(pend::locate(&task, token).is_some());
    meta.control.busy_for(0);
    assert_eq!(release_without_allocating(&task, token), Ok(1));
}

fn rooted(task: &Arc<Task>, meta: Mail, permission: Permission) -> PieToken {
    let pie = pie(meta, permission);
    let token = pie.token();
    pend::insert(task, pie).unwrap();
    token
}

#[test]
fn derive_range_wire_roundtrips_and_rejects_invalid_ranges() {
    use env::{Decode, EnvCall, GrantRange, PieCall};

    let _: fn(PieToken, Permission, Mark, GrantRange) -> env::PieResult<PieToken> =
        env::pie::derive;
    let token = PieToken::mint(42);
    let call = PieCall::Derive {
        src: token,
        subset: rights(),
        mark: Mark::NONE,
        range: GrantRange::Pages {
            offset: 2,
            count: 3,
        },
    };
    assert_eq!(call.slot(), (7usize << 32) | 13);
    assert_eq!(
        EnvCall::from_wire(call.slot(), &call.pack()),
        Ok(EnvCall::Pie(call))
    );
    for slot in [11, 12] {
        assert_eq!(
            EnvCall::from_wire((7usize << 32) | slot, &[0; 6]),
            Err(Decode::BadSlot),
            "slot {slot} must stay unassigned"
        );
    }

    let inherit = PieCall::Derive {
        src: token,
        subset: rights(),
        mark: Mark::NONE,
        range: GrantRange::Inherit,
    };
    assert_eq!(
        EnvCall::from_wire(inherit.slot(), &inherit.pack()),
        Ok(EnvCall::Pie(inherit))
    );
    assert_eq!(&inherit.pack()[3..], &[0, 0, 0]);

    let mut registers = call.pack();
    for invalid in [
        (2usize, 2usize, 3usize),
        (0, 2, 3),
        (1, 1, 0),
        (1, usize::MAX, 2),
        (1, 0, usize::MAX),
    ] {
        (registers[3], registers[4], registers[5]) = invalid;
        assert_eq!(
            EnvCall::from_wire(call.slot(), &registers),
            Err(Decode::Invalid),
            "invalid GrantRange {invalid:?}"
        );
    }
    assert!(GrantRange::pages(0, 0).is_none());
    assert!(GrantRange::pages(usize::MAX, 2).is_none());
    assert!(GrantRange::pages(0, usize::MAX).is_none());
}

#[test]
fn local_derive_inherits_exact_parent_window_and_only_narrows() {
    use crate::core::work::mail::PoleMeta;
    use crate::memory::address::{PageCount, PageOffset};

    let owner = Task::new();
    let peer = Task::new();
    let root = rooted(&owner, Mail::Pole(PoleMeta::new(owner.ident.id)), rights());
    let branch = PieToken::mint(
        pend::derive(
            &owner,
            root,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(1), PageCount::new(3).unwrap())),
        )
        .unwrap(),
    );
    let child = PieToken::mint(
        pend::accord(
            &owner,
            branch,
            &Arc::downgrade(&peer),
            rights(),
            Mark::NONE,
        )
        .unwrap(),
    );
    let inherited = PieToken::mint(pend::derive(&peer, child, rights(), Mark::NONE, None).unwrap());
    let inherited_pie = pend::locate(&peer, inherited).unwrap();
    assert_eq!(
        inherited_pie.grant().window(),
        Some((PageOffset::new(1), PageCount::new(3).unwrap()))
    );
    assert_eq!(inherited_pie.sire(), Some(child));
    assert!(inherited_pie.lord().ptr_eq(&Arc::downgrade(&peer)));

    assert_eq!(
        pend::derive(
            &peer,
            child,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(0), PageCount::new(1).unwrap())),
        ),
        Err(PieFail::Denied),
        "a local derive cannot widen outside its parent window"
    );
    let narrow = PieToken::mint(
        pend::derive(
            &peer,
            inherited,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(2), PageCount::new(1).unwrap())),
        )
        .unwrap(),
    );
    assert_eq!(
        pend::locate(&peer, narrow).unwrap().grant().window(),
        Some((PageOffset::new(2), PageCount::new(1).unwrap()))
    );
    assert_eq!(
        pend::derive(
            &peer,
            narrow,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(1), PageCount::new(2).unwrap())),
        ),
        Err(PieFail::Denied),
        "nested local derives stay inside the narrowed window"
    );
    let again = PieToken::mint(pend::derive(&peer, narrow, rights(), Mark::NONE, None).unwrap());
    assert_eq!(
        pend::locate(&peer, again).unwrap().grant().window(),
        Some((PageOffset::new(2), PageCount::new(1).unwrap()))
    );
    let transferred = grant(&peer, narrow, &owner).unwrap();
    assert_eq!(
        pend::locate(&owner, transferred).unwrap().grant().window(),
        Some((PageOffset::new(2), PageCount::new(1).unwrap()))
    );
    assert_eq!(pend::revoke(&peer, &Arc::downgrade(&peer), narrow), Ok(3));
    assert!(pend::locate(&peer, again).is_none());
    assert!(pend::locate(&owner, transferred).is_none());
    assert!(pend::locate(&peer, child).is_some());
}

#[test]
fn local_derive_rejects_missing_vest_escalation_and_non_pole_pages() {
    use crate::core::work::mail::{NoleMeta, PoleMeta};
    use crate::memory::address::{PageCount, PageOffset};

    let task = Task::new();
    let token = rooted(&task, Mail::Pole(PoleMeta::new(task.ident.id)), rights());
    let no_vest = rights() & !Permission::VEST;
    let child = PieToken::mint(pend::derive(&task, token, no_vest, Mark::NONE, None).unwrap());
    assert_eq!(pend::locate(&task, child).unwrap().permission(), no_vest);
    assert_eq!(
        pend::derive(&task, child, no_vest, Mark::NONE, None),
        Err(PieFail::Denied),
        "a derived token without VEST cannot derive again"
    );
    assert_eq!(
        pend::derive(&task, token, rights() | Permission::ONLY, Mark::NONE, None),
        Err(PieFail::Denied),
        "a subset cannot add ONLY"
    );
    let nole = rooted(&task, Mail::Nole(NoleMeta::new(task.ident.id)), rights());
    assert_eq!(
        pend::derive(
            &task,
            nole,
            rights(),
            Mark::NONE,
            Some((PageOffset::ZERO, PageCount::new(1).unwrap())),
        ),
        Err(PieFail::Denied),
        "a non-Pole resource has no page window to derive"
    );
}

#[test]
fn local_only_derive_hands_over_and_revoke_restores_source() {
    use crate::core::work::mail::PoleMeta;

    let task = Task::new();
    let permission = rights() | Permission::ONLY;
    let token = rooted(&task, Mail::Pole(PoleMeta::new(task.ident.id)), permission);
    let child = PieToken::mint(pend::derive(&task, token, permission, Mark::NONE, None).unwrap());
    let source = pend::locate(&task, token).unwrap();
    assert_eq!(source.heir().map(|heir| heir.token), Some(child));
    assert_eq!(pend::available(&source), Err(PieFail::HandedOver));
    assert_eq!(
        pend::derive(&task, token, permission, Mark::NONE, None),
        Err(PieFail::HandedOver)
    );

    assert_eq!(pend::revoke(&task, &Arc::downgrade(&task), child), Ok(1));
    assert!(pend::locate(&task, child).is_none());
    let restored = pend::locate(&task, token).unwrap();
    assert_eq!(pend::available(&restored), Ok(()));
    assert!(pend::derive(&task, token, permission, Mark::NONE, None).is_ok());
}

#[test]
fn revoking_local_derived_parent_removes_descendants_but_not_siblings() {
    use crate::core::work::mail::PoleMeta;
    use crate::memory::address::{PageCount, PageOffset};

    let task = Task::new();
    let token = rooted(&task, Mail::Pole(PoleMeta::new(task.ident.id)), rights());
    let first = PieToken::mint(
        pend::derive(
            &task,
            token,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(1), PageCount::new(2).unwrap())),
        )
        .unwrap(),
    );
    let sibling = PieToken::mint(
        pend::derive(
            &task,
            token,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(4), PageCount::new(2).unwrap())),
        )
        .unwrap(),
    );
    let descendant = PieToken::mint(
        pend::derive(
            &task,
            first,
            rights(),
            Mark::NONE,
            Some((PageOffset::new(1), PageCount::new(1).unwrap())),
        )
        .unwrap(),
    );

    assert_eq!(pend::revoke(&task, &Arc::downgrade(&task), first), Ok(2));
    assert!(pend::locate(&task, first).is_none());
    assert!(pend::locate(&task, descendant).is_none());
    assert!(pend::locate(&task, sibling).is_some());
    assert!(pend::locate(&task, token).is_some());
}
