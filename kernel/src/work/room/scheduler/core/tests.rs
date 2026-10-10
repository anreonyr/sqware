use super::*;

#[cfg(debug_assertions)]
pub fn acceptance() {
    let _commit = crate::work::unit::commit();
    use crate::hart::HartId;
    use crate::work::unit::{space::SpaceBuilder, team::TeamBuilder};

    let space = SpaceBuilder::user().build().unwrap();
    space.with(|inner| inner.dynamic(0x4000_0000));
    let team = TeamBuilder::new(space).spawn().unwrap();
    let mut tasks: alloc::vec::Vec<_> = (0..4).map(|_| team.task().hold().unwrap()).collect();
    for task in &mut tasks {
        task.as_ref().transform(TaskState::Starved { next: None });
    }
    let victim = Scheduler::new(HartId::new(0));
    assert!(victim.push(tasks[0].clone()));
    assert!(!victim.push(tasks[1].clone()));
    assert!(!victim.push(tasks[2].clone()));
    {
        let mut i = victim.inner.lock();
        assert!(victim.steal().is_none());
        assert!(i.ready_ceiling() <= clock::duration_to_ticks(Duration::from_millis(READY_MS)));
        i.ready_since = Some(
            clock::now()
                .as_ticks()
                .saturating_sub(clock::duration_to_ticks(Duration::from_millis(READY_MS))),
        );
        assert_eq!(i.ready_ceiling(), 0);
        assert!(victim.starved_remove(&mut i, &tasks[1]));
    }
    assert!(Arc::ptr_eq(&victim.steal().unwrap(), &tasks[0]));
    assert!(Arc::ptr_eq(&victim.pull().unwrap(), &tasks[2]));
    assert!(victim.steal().is_none());
    assert_eq!(victim.inner.lock().ready_ceiling(), timer::blind_ceiling());

    *tasks[0].state.lock() = TaskState::Debarked { state: crate::work::unit::task::TaskStopped::Starved };
    assert!(victim.push(tasks[3].clone()));
    assert!(Arc::ptr_eq(&victim.steal().unwrap(), &tasks[3]));
    assert!(victim.pull().is_none());
    assert!(tasks[0].stopped());
    assert!(victim.push(tasks[2].clone()));
    {
        let mut i = victim.inner.lock();
        assert!(victim.starved_remove(&mut i, &tasks[2]));
        assert!(i.ready_since.is_none());
        assert!(i.tail.is_none());
    }
    for task in &mut tasks {
        assert!(team.release_held(task));
        task.as_ref().transform(TaskState::Doomed { hart: None, cause: crate::work::unit::task::TaskExitCause::Slay, reason: 0 });
    }
}
