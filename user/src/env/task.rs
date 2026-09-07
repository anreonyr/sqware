//! Unit 域：`UnitCall::*` 转发（执行单元：team 建域 / task 建线程）。

use ubi::{TeamId, TaskId, UnitCall, UnitCallRet, EnvResult};

/// 在**当前** team 里建线程。
pub fn spawn(entry: usize, arg: usize, stack: usize) -> EnvResult<TaskId> {
    let r = UnitCall::Spawn { entry, arg, stack }.call()?;
    match r {
        UnitCallRet::Spawn(id) => Ok(id),
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

/// 溯源：生我者的 task id（0 = 顶级域 / 父已亡）。
pub fn sire() -> EnvResult<TaskId> {
    let r = UnitCall::Sire.call()?;
    match r {
        UnitCallRet::Sire(id) => Ok(id),
        _ => unreachable!(),
    }
}

/// 我生的子域数量（heir 枚举 first pass）。
pub fn heir_count() -> EnvResult<usize> {
    let r = UnitCall::HeirCount.call()?;
    match r {
        UnitCallRet::HeirCount(n) => Ok(n),
        _ => unreachable!(),
    }
}

/// 按索引取子域 TeamId（heir 枚举 second pass；越界 → 0）。
pub fn heir_at(index: usize) -> EnvResult<TeamId> {
    let r = UnitCall::Heir { index }.call()?;
    match r {
        UnitCallRet::Heir(id) => Ok(id),
        _ => unreachable!(),
    }
}
