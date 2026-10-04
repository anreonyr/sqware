extern crate alloc;
#[path = "../mod.rs"]
mod schedule;

#[cfg(test)]
mod tests {
    use super::schedule::{*, resource::AccessError};
    #[derive(Default)] struct Trace(Vec<&'static str>);
    #[derive(Default)] struct Gate(bool);
    fn pre(mut trace: ResMut<Trace>) -> Result<Progress, &'static str> { trace.0.push("pre"); Ok(Progress::Done) }
    fn wait(gate: Res<Gate>, mut trace: ResMut<Trace>) -> Result<Progress, &'static str> {
        trace.0.push("wait");
        Ok(if gate.0 { Progress::Done } else { Progress::Pending })
    }
    fn post(mut trace: ResMut<Trace>) -> Result<Progress, &'static str> { trace.0.push("post"); Ok(Progress::Done) }
    fn fail(_: Res<Trace>) -> Result<Progress, &'static str> { Err("failed") }
    fn alias(_: ResMut<Trace>, _: Res<Trace>) -> Result<Progress, &'static str> { Ok(Progress::Done) }
    fn resource_set() -> Resources<'static> {
        let mut resources = Resources::new(); resources.insert(Trace::default()).unwrap(); resources.insert(Gate::default()).unwrap(); resources
    }
    #[test] fn pending_resumes_and_releases_borrows() {
        let mut schedule = Schedule::new();
        schedule.add_system("post", Phase::PostMint, post).unwrap();
        schedule.add_system("wait", Phase::Mint, wait).unwrap();
        schedule.add_system("pre", Phase::PreMint, pre).unwrap();
        let mut plan = schedule.build().unwrap(); let resources = resource_set(); let mut cursor = Cursor::default();
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Pending));
        resources.write::<Gate>().unwrap().0 = true;
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Done));
        assert_eq!(resources.read::<Trace>().unwrap().0, ["pre", "wait", "wait", "post"]);
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Done));
        assert_eq!(resources.read::<Trace>().unwrap().0.len(), 4);
    }
    #[test] fn failure_does_not_execute_post() {
        let mut schedule = Schedule::new(); schedule.add_system("pre", Phase::PreMint, pre).unwrap();
        schedule.add_system("fail", Phase::Mint, fail).unwrap(); schedule.add_system("post", Phase::PostMint, post).unwrap();
        let resources = resource_set();
        assert_eq!(schedule.build().unwrap().advance(&mut Cursor::default(), &resources), Err(RunError::Step("failed")));
        assert_eq!(resources.read::<Trace>().unwrap().0, ["pre"]);
    }
    #[test] fn explicit_edges_order_same_phase() {
        let mut schedule = Schedule::new(); schedule.add_system("a", Phase::Mint, post).unwrap();
        schedule.add_system("z", Phase::Mint, pre).unwrap(); schedule.before("z", "a").unwrap();
        let resources = resource_set(); schedule.build().unwrap().advance(&mut Cursor::default(), &resources).unwrap();
        assert_eq!(resources.read::<Trace>().unwrap().0, ["pre", "post"]);
    }
    #[test] fn cycle_rejected() {
        let mut schedule = Schedule::new(); schedule.add_system("a", Phase::Mint, pre).unwrap(); schedule.add_system("b", Phase::Mint, post).unwrap();
        schedule.before("a", "b").unwrap(); schedule.before("b", "a").unwrap(); assert!(matches!(schedule.build(), Err(BuildError::Cycle)));
    }
    #[test] fn backwards_phase_edge_rejected() {
        let mut schedule = Schedule::new(); schedule.add_system("pre", Phase::PreMint, pre).unwrap(); schedule.add_system("post", Phase::PostMint, post).unwrap();
        schedule.before("post", "pre").unwrap(); assert!(matches!(schedule.build(), Err(BuildError::Cycle)));
    }
    #[test] fn unknown_dependency_rejected() {
        let mut schedule = Schedule::new(); schedule.add_system("pre", Phase::PreMint, pre).unwrap(); schedule.before("missing", "pre").unwrap();
        assert!(matches!(schedule.build(), Err(BuildError::Unknown)));
    }
    #[test] fn duplicate_and_alias_rejected() {
        let mut schedule = Schedule::new(); schedule.add_system("pre", Phase::PreMint, pre).unwrap();
        assert_eq!(schedule.add_system("pre", Phase::Mint, pre), Err(BuildError::Duplicate));
        assert_eq!(schedule.add_system("alias", Phase::Mint, alias), Err(BuildError::BorrowConflict));
    }
    #[test] fn borrowed_resource_updates_owner() {
        let mut trace = Trace::default();
        { let mut resources = Resources::new(); resources.borrow(&mut trace).unwrap();
          let mut schedule = Schedule::new(); schedule.add_system("pre", Phase::PreMint, pre).unwrap();
          schedule.build().unwrap().advance(&mut Cursor::default(), &resources).unwrap(); }
        assert_eq!(trace.0, ["pre"]);
    }
    #[test] fn missing_resource_is_execution_error() {
        let mut schedule = Schedule::new(); schedule.add_system("pre", Phase::PreMint, pre).unwrap();
        assert_eq!(schedule.build().unwrap().advance(&mut Cursor::default(), &Resources::new()), Err(RunError::Resource(AccessError::Missing)));
    }
    #[test] fn simultaneous_borrows_checked() {
        let resources = resource_set(); let read = resources.read::<Trace>().unwrap();
        assert!(matches!(resources.write::<Trace>(), Err(AccessError::Borrowed))); drop(read);
        assert!(resources.write::<Trace>().is_ok());
    }
    #[test] fn readonly_resource_cannot_be_mutated() {
        let gate = Gate(true); let mut resources = Resources::new(); resources.observe(&gate).unwrap();
        assert!(resources.read::<Gate>().unwrap().0); assert!(resources.write::<Gate>().is_err());
    }

    #[derive(Default)] struct Queue(alloc::collections::VecDeque<(u8, Cursor)>);
    #[derive(Default)] struct Calls { picks: usize, finishes: usize, fail_once: bool }
    fn round(queue: Res<Queue>, mut dispatch: ResMut<Dispatch<u8, &'static str>>) -> Result<Progress, &'static str> { dispatch.budget = queue.0.len(); Ok(Progress::Done) }
    fn choose(mut queue: ResMut<Queue>, mut dispatch: ResMut<Dispatch<u8, &'static str>>, mut calls: ResMut<Calls>) -> Result<Progress, &'static str> {
        calls.picks += 1;
        dispatch.current = queue.0.pop_front().map(|(key, cursor)| Invocation { key, cursor }); Ok(Progress::Done)
    }
    fn finish(mut queue: ResMut<Queue>, mut dispatch: ResMut<Dispatch<u8, &'static str>>, mut calls: ResMut<Calls>) -> Result<Progress, &'static str> {
        calls.finishes += 1;
        let mut invocation = dispatch.current.take().unwrap();
        match dispatch.result.take().unwrap() {
            Ok(Progress::Done) => {},
            Ok(Progress::Pending) => queue.0.push_back((invocation.key, invocation.cursor)),
            Err(RunError::Step("failed")) => { invocation.cursor.reset(); queue.0.push_back((1, invocation.cursor)); },
            Err(_) => return Err("unexpected error"),
        }
        Ok(Progress::Done)
    }
    fn dispatcher(children: Vec<(u8, Plan<&'static str>)>) -> Plan<&'static str> {
        let mut parent = Schedule::new(); parent.add_system("round", 0u8, round).unwrap();
        parent.add_subplans("children", 1, choose, children, finish).unwrap(); parent.build().unwrap()
    }
    fn queued(keys: &[u8]) -> Resources<'static> {
        let mut resources = resource_set();
        resources.insert(Queue(keys.iter().map(|key| (*key, Cursor::default())).collect())).unwrap();
        resources.insert(Dispatch::<u8, &'static str>::new()).unwrap(); resources.insert(Calls::default()).unwrap(); resources
    }
    fn waiting() -> Plan<&'static str> {
        let mut plan = Schedule::new(); plan.add_system("pre", 0u8, pre).unwrap(); plan.add_system("wait", 1, wait).unwrap(); plan.add_system("post", 2, post).unwrap(); plan.build().unwrap()
    }
    #[test] fn fixed_nested_plan_resumes_and_releases_borrows() {
        let resources = resource_set(); let mut parent = Schedule::new();
        parent.add_plan("child", 0u8, waiting()).unwrap(); parent.add_system("after", 1, post).unwrap();
        let mut plan = parent.build().unwrap(); let mut cursor = Cursor::default();
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Pending));
        resources.write::<Gate>().unwrap().0 = true;
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Done));
        assert_eq!(resources.read::<Trace>().unwrap().0, ["pre", "wait", "wait", "post", "post"]);
    }
    #[test] fn pending_subplans_rotate_once_per_round_and_resume_their_cursor() {
        let resources = queued(&[0, 0, 0]); let mut plan = dispatcher(alloc::vec![(0, waiting())]);
        assert_eq!(plan.advance(&mut Cursor::default(), &resources), Ok(Progress::Done));
        assert_eq!(resources.read::<Queue>().unwrap().0.len(), 3);
        assert_eq!(resources.read::<Calls>().unwrap().picks, 3);
        assert_eq!(resources.read::<Calls>().unwrap().finishes, 3);
        resources.write::<Gate>().unwrap().0 = true;
        assert_eq!(plan.advance(&mut Cursor::default(), &resources), Ok(Progress::Done));
        assert!(resources.read::<Queue>().unwrap().0.is_empty());
        let trace = resources.read::<Trace>().unwrap();
        assert_eq!(trace.0.iter().filter(|event| **event == "pre").count(), 3);
        assert_eq!(trace.0.iter().filter(|event| **event == "post").count(), 3);
    }
    #[test] fn child_failure_is_delivered_to_finisher_and_compensation_runs_next_round() {
        let resources = queued(&[0]); let mut failed = Schedule::new(); failed.add_system("fail", 0u8, fail).unwrap();
        let mut compensate = Schedule::new(); compensate.add_system("compensate", 0u8, post).unwrap();
        let mut plan = dispatcher(alloc::vec![(0, failed.build().unwrap()), (1, compensate.build().unwrap())]);
        assert_eq!(plan.advance(&mut Cursor::default(), &resources), Ok(Progress::Done));
        assert_eq!(resources.read::<Queue>().unwrap().0.front().unwrap().0, 1);
        assert!(resources.read::<Trace>().unwrap().0.is_empty());
        plan.advance(&mut Cursor::default(), &resources).unwrap();
        assert!(resources.read::<Queue>().unwrap().0.is_empty()); assert_eq!(resources.read::<Trace>().unwrap().0, ["post"]);
    }
    #[test] fn unknown_subplan_key_is_a_scheduler_error() {
        let resources = queued(&[3]); let mut plan = dispatcher(alloc::vec![(0, waiting())]);
        assert_eq!(plan.advance(&mut Cursor::default(), &resources), Err(RunError::UnknownPlan));
        assert_eq!(resources.read::<Calls>().unwrap().finishes, 0);
    }
    fn pause_finish(mut calls: ResMut<Calls>, mut dispatch: ResMut<Dispatch<u8, &'static str>>) -> Result<Progress, &'static str> {
        calls.finishes += 1;
        if calls.finishes == 1 { return Ok(Progress::Pending); }
        assert_eq!(dispatch.result.take(), Some(Ok(Progress::Done))); dispatch.current = None; Ok(Progress::Done)
    }
    #[test] fn pending_finisher_resumes_without_selecting_or_advancing_child_again() {
        let resources = queued(&[0]); let mut child = Schedule::new(); child.add_system("post", 0u8, post).unwrap();
        let mut parent = Schedule::new(); parent.add_system("round", 0u8, round).unwrap();
        parent.add_subplans("children", 1, choose, alloc::vec![(0u8, child.build().unwrap())], pause_finish).unwrap();
        let mut plan = parent.build().unwrap(); let mut cursor = Cursor::default();
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Pending));
        assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Done));
        assert_eq!(resources.read::<Calls>().unwrap().picks, 1); assert_eq!(resources.read::<Trace>().unwrap().0, ["post"]);
    }
    fn child_borrow(mut queue: ResMut<Queue>, mut calls: ResMut<Calls>) -> Result<Progress, &'static str> { calls.fail_once = true; queue.0.clear(); Ok(Progress::Done) }
    #[test] fn select_child_and_finish_release_shared_resource_borrows() {
        let resources = queued(&[0]); let mut child = Schedule::new(); child.add_system("borrow", 0u8, child_borrow).unwrap();
        let mut plan = dispatcher(alloc::vec![(0, child.build().unwrap())]);
        assert_eq!(plan.advance(&mut Cursor::default(), &resources), Ok(Progress::Done));
        assert!(resources.read::<Calls>().unwrap().fail_once); assert_eq!(resources.read::<Calls>().unwrap().finishes, 1);
    }
    #[test] fn subplans_reject_duplicate_keys_and_conflicting_selection_borrows() {
        let mut parent = Schedule::new();
        assert_eq!(parent.add_subplans("duplicate", 0u8, choose, alloc::vec![(0u8, waiting()), (0, waiting())], finish), Err(BuildError::Duplicate));
        assert_eq!(parent.add_subplans("alias", 0u8, alias, alloc::vec![(0u8, waiting())], finish), Err(BuildError::BorrowConflict));
    }
    #[test] fn child_resource_errors_are_delivered_without_erasing_their_kind() {
        fn resource_error(mut dispatch: ResMut<Dispatch<u8, &'static str>>) -> Result<Progress, &'static str> {
            assert_eq!(dispatch.result.take(), Some(Err(RunError::Resource(AccessError::Missing)))); dispatch.current = None; Ok(Progress::Done)
        }
        let resources = queued(&[0]);
        fn missing(_: Res<u64>) -> Result<Progress, &'static str> { Ok(Progress::Done) }
        let mut child = Schedule::new(); child.add_system("missing", 0u8, missing).unwrap();
        let mut parent = Schedule::new(); parent.add_system("round", 0u8, round).unwrap();
        parent.add_subplans("child", 1, choose, alloc::vec![(0u8, child.build().unwrap())], resource_error).unwrap();
        assert_eq!(parent.build().unwrap().advance(&mut Cursor::default(), &resources), Ok(Progress::Done));
    }

    #[test] fn fixed_nested_cursor_is_owned_by_each_invocation() {
        let resources = queued(&[0, 0, 0]); let mut child = Schedule::new();
        child.add_plan("nested", 0u8, waiting()).unwrap();
        let mut plan = dispatcher(alloc::vec![(0, child.build().unwrap())]);
        plan.advance(&mut Cursor::default(), &resources).unwrap();
        assert_eq!(resources.read::<Trace>().unwrap().0.iter().filter(|event| **event == "pre").count(), 3);
        resources.write::<Gate>().unwrap().0 = true;
        plan.advance(&mut Cursor::default(), &resources).unwrap();
        assert_eq!(resources.read::<Trace>().unwrap().0.iter().filter(|event| **event == "pre").count(), 3);
        assert_eq!(resources.read::<Trace>().unwrap().0.iter().filter(|event| **event == "post").count(), 3);
    }
    #[test] fn resetting_failed_nested_cursor_restarts_validation_for_next_call() {
        let resources = resource_set(); let mut child = Schedule::new();
        child.add_system("pre", 0u8, pre).unwrap(); child.add_system("fail", 1, fail).unwrap();
        let mut parent = Schedule::new(); parent.add_plan("nested", 0u8, child.build().unwrap()).unwrap();
        let mut plan = parent.build().unwrap(); let mut cursor = Cursor::default();
        assert_eq!(plan.advance(&mut cursor, &resources), Err(RunError::Step("failed")));
        cursor.reset();
        assert_eq!(plan.advance(&mut cursor, &resources), Err(RunError::Step("failed")));
        assert_eq!(resources.read::<Trace>().unwrap().0, ["pre", "pre"]);
    }
}

#[cfg(test)]
mod allocations {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! { static COUNT: Cell<Option<usize>> = const { Cell::new(None) }; }
    pub struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = COUNT.try_with(|count| {
                if let Some(value) = count.get() {
                    count.set(Some(value + 1));
                }
            });
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let _ = COUNT.try_with(|count| {
                if let Some(value) = count.get() {
                    count.set(Some(value + 1));
                }
            });
            unsafe { System.realloc(ptr, layout, size) }
        }
    }
    pub fn count(run: impl FnOnce()) -> usize {
        COUNT.with(|count| count.set(Some(0)));
        run();
        COUNT.with(|count| count.replace(None).unwrap())
    }
}
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: allocations::Counting = allocations::Counting;

#[cfg(test)]
mod regression {
    use super::{allocations, schedule::*};
    #[derive(Default)]
    struct Counter(usize);
    fn increment(mut counter: ResMut<Counter>) -> Result<Progress, &'static str> {
        counter.0 += 1;
        Ok(Progress::Done)
    }
    #[test]
    fn nested_mapped_plans_allocate_nothing_after_warmup() {
        let mut resources = Resources::new();
        resources.insert(Counter::default()).unwrap();
        let mut child = Schedule::new();
        child.add_system("increment", 0u8, increment).unwrap();
        let mut parent = Schedule::new();
        parent
            .add_plan(
                "first",
                0u8,
                child.build().unwrap().map_error(|error| error),
            )
            .unwrap();
        let mut second = Schedule::new();
        second.add_system("increment", 0u8, increment).unwrap();
        parent
            .add_plan("second", 1, second.build().unwrap())
            .unwrap();
        let mut plan = parent.build().unwrap();
        plan.prepare(&resources);
        let mut cursor = Cursor::default();
        plan.advance(&mut cursor, &resources).unwrap();
        let allocations = allocations::count(|| {
            for _ in 0..1000 {
                cursor.reset();
                assert_eq!(plan.advance(&mut cursor, &resources), Ok(Progress::Done));
            }
        });
        assert_eq!(
            allocations, 0,
            "resetting a completed nested plan must reuse its storage"
        );
        assert_eq!(resources.read::<Counter>().unwrap().0, 2002);
    }
    #[test]
    fn cached_slots_rebind_when_resource_order_changes() {
        let mut first = Resources::new();
        first.insert(Counter::default()).unwrap();
        first.insert(false).unwrap();
        let mut second = Resources::new();
        second.insert(false).unwrap();
        second.insert(Counter(20)).unwrap();
        let mut schedule = Schedule::new();
        schedule.add_system("increment", 0u8, increment).unwrap();
        let mut plan = schedule.build().unwrap();
        plan.prepare(&first);
        plan.advance(&mut Cursor::default(), &first).unwrap();
        plan.advance(&mut Cursor::default(), &second).unwrap();
        plan.advance(&mut Cursor::default(), &first).unwrap();
        assert_eq!(first.read::<Counter>().unwrap().0, 2);
        assert_eq!(second.read::<Counter>().unwrap().0, 21);
        assert!(!*second.read::<bool>().unwrap());
    }
    #[test]
    fn preparing_unused_children_does_not_raise_their_missing_resource_error() {
        fn choose(
            mut dispatch: ResMut<Dispatch<u8, &'static str>>,
        ) -> Result<Progress, &'static str> {
            dispatch.current = Some(Invocation {
                key: 0,
                cursor: Cursor::default(),
            });
            Ok(Progress::Done)
        }
        fn finish(
            mut dispatch: ResMut<Dispatch<u8, &'static str>>,
        ) -> Result<Progress, &'static str> {
            assert_eq!(dispatch.result.take(), Some(Ok(Progress::Done)));
            dispatch.current = None;
            Ok(Progress::Done)
        }
        fn done() -> Result<Progress, &'static str> {
            Ok(Progress::Done)
        }
        let mut present = Schedule::new();
        present.add_system("done", 0u8, done).unwrap();
        let mut absent = Schedule::new();
        absent.add_system("missing", 0u8, increment).unwrap();
        let mut parent = Schedule::new();
        parent
            .add_subplans(
                "select",
                0u8,
                choose,
                alloc::vec![
                    (0u8, present.build().unwrap()),
                    (1, absent.build().unwrap())
                ],
                finish,
            )
            .unwrap();
        let mut resources = Resources::new();
        let mut dispatch = Dispatch::<u8, &'static str>::new();
        dispatch.budget = 1;
        resources.insert(dispatch).unwrap();
        let mut plan = parent.build().unwrap();
        plan.prepare(&resources);
        assert_eq!(
            plan.advance(&mut Cursor::default(), &resources),
            Ok(Progress::Done)
        );
    }
}
