//! Unit 域：`UnitCall::*` 转发（执行单元：team 建域 / task 建线程）。

use ubi::{Spawnee, TeamId, TaskId, UnitCall, UnitCallRet, EnvResult};

/// 在**当前** team 里建线程。
pub fn spawn(entry: usize, arg: usize, stack: usize) -> EnvResult<TaskId> {
    let r = UnitCall::Spawn { entry, arg, stack }.call()?;
    match r {
        UnitCallRet::Spawn(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 装载镜像成独立域（建 Space+Team，不产 task）。返 TeamId。
pub fn spawn_team(which: Spawnee) -> EnvResult<TeamId> {
    let r = UnitCall::SpawnTeam { which }.call()?;
    match r {
        UnitCallRet::SpawnTeam(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 在给定 team 下建线程（域内产 task）。返 TaskId。
pub fn spawn_task(team: TeamId, entry: usize, arg: usize) -> EnvResult<TaskId> {
    let r = UnitCall::SpawnTask { team, entry, arg }.call()?;
    match r {
        UnitCallRet::SpawnTask(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 溯源：生我者的 task id（`spawn_team` 子域才有；0 = 顶级域 / 父已亡）。
pub fn sire() -> EnvResult<TaskId> {
    let r = UnitCall::Sire.call()?;
    match r {
        UnitCallRet::Sire(id) => Ok(id),
        _ => unreachable!(),
    }
}
