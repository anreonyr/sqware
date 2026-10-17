use super::*;

/// 测试装置：把一枚任务放上本核运行槽，跑完再把它拿下来。
///
/// `Pie` / `Mail` 这些入口的调用者取自 `current().running_task()`，而其余健康用例
/// 只经 `ident` 走线外调用面。这里借运行槽自己那一手（`Conductor::seat`）把任务真的
/// 装上去，不另造一套“假装在座”的状态。
///
/// **装入做四件事**：轴换成 `OnHart{本核, QUANTUM_TICKS}`；把 `frame.kernel_sp` 指向
/// 本核的 trap 栈顶（内核空间还要把 `TP` 指向本核的 `PerHart`）；徽记换成这一枚的活
/// `TaskIdent`；按本核饥饿队列重上时限（`deadline::beat_until(ready_ceiling)`）。
///
/// **拿下来只做两件**：运行槽腾空、徽记摘掉，再把轴放回 `Ready`——`frame.kernel_sp`、
/// `TP` 与时限都**不**还原（本装置不使用它们；下一次装槽会重设）。若闭包把这一枚判死
/// 或摘下了轴，就不换轴，免得把它“复活”成 `Ready`。
///
/// 任务须是 `Held`（刚装配好：此时先从团队暂扣表里依法放出来，与 `Task::release`
/// 同一手，只是**不** `launch` 到本核的饥饿队列）或 `Ready`（上次放下之后）；
/// 本核须本来空着。
pub fn with_running<T>(task: Arc<Task>, run: impl FnOnce() -> T) -> T {
    let conductor = current();
    assert!(
        conductor.running_task().is_none(),
        "with_running: the hart already has a running task"
    );
    if task.held() {
        assert!(
            task.ident.team.release_held(&task),
            "with_running: a held task is in the team's held ledger"
        );
        task.transform_checked(TaskState::ready());
    }
    assert!(task.ready(), "with_running: the task must be held or ready");
    assert!(
        conductor.seat(task.clone()).is_some(),
        "with_running: a ready, unpaused task must seat"
    );
    let result = run();
    {
        let mut inner = conductor.inner.lock();
        let seated = inner.running.take().expect("with_running: still seated");
        drop(inner);
        conductor.badge.shed(&seated.ident);
        if !seated.dead() && seated.on_hart().is_some() {
            seated.transform_checked(TaskState::ready());
        }
    }
    result
}

#[cfg(debug_assertions)]
pub fn acceptance() {
    let _commit = crate::core::work::commit();
    use crate::core::work::unit::{space::SpaceBuilder, team::TeamBuilder};
    use crate::machine::hart::HartId;

    let space = SpaceBuilder::user().build().unwrap();
    space
        .nest
        .attach_region(
            crate::core::memory::nest::RegionId::Normal,
            0x4000_0000,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let mut tasks: alloc::vec::Vec<_> = (0..4).map(|_| team.task().hold().unwrap()).collect();
    for task in &mut tasks {
        task.as_ref().transform_checked(TaskState::ready());
    }
    let victim = Conductor::new(HartId::new(0));
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
    assert_eq!(
        victim.inner.lock().ready_ceiling(),
        deadline::blind_ceiling()
    );

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
