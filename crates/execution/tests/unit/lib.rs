#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
pub use abi::{TaskId, TeamId, UnitResult, UnitFail, Wait, EXIT_OK};
use std::collections::BTreeMap;
use std::sync::Mutex;
#[derive(Default)]
struct Effects { next: usize, spawn_fail: bool, embark_fail: bool, prune_slain: bool, tasks: BTreeMap<usize, (usize, bool)> }
static EFFECTS: Mutex<Effects> = Mutex::new(Effects {
    next: 0, spawn_fail: false, embark_fail: false, prune_slain: false, tasks: BTreeMap::new(),
});
#[path = "../../src/lock.rs"] pub mod lock;
#[path = "../../src/unit/task.rs"] pub mod task;
mod tls {
    pub fn allocate() -> Result<usize, ()> { Ok(1) }
    pub fn deallocate() {}
}
pub mod room {
    pub fn wake(_: usize) -> Result<(), ()> { Ok(()) }
    pub fn wait(_: usize, _: crate::Wait) -> Result<(), ()> {
        let next = crate::EFFECTS.lock().unwrap().tasks.iter()
            .find(|(_, (_, done))| !done).map(|(id, _)| *id);
        if let Some(id) = next { crate::run(crate::TaskId::new(id)); }
        Ok(())
    }
    pub fn reap(_: usize, _: Option<&str>) -> ! { panic!("unexpected trampoline exit") }
}
pub mod unit {
    pub fn self_id() -> crate::TaskId { crate::TaskId::new(1) }
    pub fn embark_task(_: crate::TaskId) -> crate::UnitResult<()> {
        if crate::EFFECTS.lock().unwrap().embark_fail {
            Err(erra::Error::new("mock", crate::UnitFail::Busy))
        } else { Ok(()) }
    }
    pub fn slay_task(id: crate::TaskId) -> crate::UnitResult<()> {
        let mut effects = crate::EFFECTS.lock().unwrap();
        if effects.prune_slain { effects.tasks.remove(&id.get()); }
        else { effects.tasks.get_mut(&id.get()).unwrap().1 = true; }
        Ok(())
    }
    pub fn join_task(id: crate::TaskId, _: crate::Wait) -> crate::UnitResult<bool> {
        match crate::EFFECTS.lock().unwrap().tasks.get(&id.get()) {
            Some((_, done)) => Ok(*done),
            None => Err(erra::Error::new("pruned same-team member", crate::UnitFail::Denied)),
        }
    }
}
pub fn spawn(_: TeamId, _: usize, args: &[usize], _: usize) -> UnitResult<TaskId> {
    let mut effects = EFFECTS.lock().unwrap();
    if effects.spawn_fail { return Err(erra::Error::new("mock", UnitFail::OoM)); }
    effects.next += 1;
    let id = effects.next;
    effects.tasks.insert(id, (args[0], false));
    Ok(TaskId::new(id))
}
fn run(id: TaskId) {
    let pointer = EFFECTS.lock().unwrap().tasks.get(&id.get()).unwrap().0;
    let holder: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(pointer as *mut _) };
    holder();
    EFFECTS.lock().unwrap().tasks.get_mut(&id.get()).unwrap().1 = true;
}
#[test]
fn closure_results_failure_rollback_and_early_drop_follow_native_lifetime() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    struct Count(Arc<AtomicUsize>);
    impl Drop for Count { fn drop(&mut self) { self.0.fetch_add(1, Ordering::Relaxed); } }
    let count = Arc::new(AtomicUsize::new(0));
    EFFECTS.lock().unwrap().spawn_fail = true;
    let captured = Count(count.clone());
    assert!(task::try_spawn(move || { drop(captured); 1 }).is_err());
    assert_eq!(count.load(Ordering::Relaxed), 1);
    EFFECTS.lock().unwrap().spawn_fail = false;
    EFFECTS.lock().unwrap().embark_fail = true;
    let captured = Count(count.clone());
    assert!(task::try_spawn(move || { drop(captured); 1 }).is_err());
    assert_eq!(count.load(Ordering::Relaxed), 2);
    EFFECTS.lock().unwrap().prune_slain = true;
    let pruned = Arc::new(AtomicUsize::new(0));
    let captured = Count(pruned.clone());
    assert!(task::try_spawn(move || { drop(captured); 1 }).is_err());
    assert_eq!(pruned.load(Ordering::Relaxed), 1);
    EFFECTS.lock().unwrap().prune_slain = false;
    EFFECTS.lock().unwrap().embark_fail = false;
    let result = Count(count.clone());
    let join = task::spawn(move || result);
    let id = join.id();
    drop(join);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    run(id);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    // Starting the next closure sweeps the detached terminal Rust value.
    let join = task::spawn(|| 42);
    assert_eq!(count.load(Ordering::Relaxed), 3);
    assert_eq!(join.join(), 42);
    // A child can exit before publishing DONE. No invented successful T.
    let join = task::spawn(|| 99);
    let id = join.id();
    let (pointer, _) = EFFECTS.lock().unwrap().tasks.remove(&id.get()).unwrap();
    unsafe { drop(Box::from_raw(pointer as *mut Box<dyn FnOnce() + Send>)); }
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| join.join())).is_err());
}
