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
}
