use super::*;

#[cfg(debug_assertions)]
pub fn acceptance() {
    let _commit = crate::core::work::unit::commit();
    use crate::machine::hart::HartId;
    use crate::core::work::unit::{space::SpaceBuilder, team::TeamBuilder};

    let space = SpaceBuilder::user().build().unwrap();
    space
        .memory
        .attach_region(
            crate::core::memory::manager::RegionId::Normal,
            0x4000_0000,
            crate::core::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let mut tasks: alloc::vec::Vec<_> = (0..4).map(|_| team.task().hold().unwrap()).collect();
    for task in &mut tasks {
        task.as_ref().transform_checked(TaskState::ready());
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
    assert_eq!(victim.inner.lock().ready_ceiling(), deadline::blind_ceiling());

    {
        let mut state = tasks[0].state.lock();
        *state = crate::core::work::unit::task::TaskState::ready();
        state.pause();
    }
    assert!(victim.push(tasks[3].clone()));
    assert!(Arc::ptr_eq(&victim.steal().unwrap(), &tasks[3]));
    assert!(victim.pull().is_none());
    assert!(tasks[0].stopped());
    assert!(victim.push(tasks[2].clone()));
    {
        let mut i = victim.inner.lock();
        assert!(victim.starved_remove(&mut i, &tasks[2]));
        assert!(i.ready_since.is_none());
        assert!(i.chain.is_empty());
    }
    for task in &mut tasks {
        assert!(team.release_held(task));
        task.state
            .lock()
            .doom(None, crate::core::work::unit::task::TaskExitCause::Slay, 0);
    }
}
