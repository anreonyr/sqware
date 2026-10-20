use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::core::memory::nest::NestError;
use crate::core::work::commit;
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::life::{Life, TeamLife};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::{Task, TaskWeak};
use crate::machine::sync::{Level, SpinLock};

use super::stage::TeamStage;
use super::state::TeamState;
use super::tasks::Tasks;
use super::{MAX_CHILDREN, MAX_DEPTH, Team};

static NEXT: AtomicUsize = AtomicUsize::new(1);

/// 下一个团队号：到 `isize::MAX` 就不再发新号（既有回绕口径）。
pub(crate) fn next_id() -> Result<TeamId, NestError> {
    let id = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            (id < isize::MAX as usize).then_some(id + 1)
        })
        .map_err(|_| NestError::OutOfMemory)?;
    Ok(TeamId::new(id))
}

/// 从零落一座团队：造状态机、成员表与生命周期线，再把自己的根挂到家长名下。
/// 家长位的深度与容量都在这里把关（失败时团队本身还没任何人看得到）。
pub(crate) fn make_team(
    space: Arc<Space>,
    sire: Option<Arc<Task>>,
    constructing: bool,
) -> Result<Arc<Team>, NestError> {
    let parent = sire.as_ref().map(|task| task.ident.team.life.clone());
    let owner = sire.as_ref().map_or(TaskId::new(0), |task| task.ident.id);
    let id = next_id()?;
    let state = Arc::try_new(SpinLock::new_level(
        Level::UnitState,
        TeamState::new(if constructing {
            TeamStage::Constructing
        } else {
            TeamStage::Ready { default_entry: 0 }
        }),
    ))
    .map_err(|_| NestError::OutOfMemory)?;
    let tasks = Arc::try_new(SpinLock::new_level(Level::TeamTasks, Vec::new()))
        .map_err(|_| NestError::OutOfMemory)?;
    let life = Arc::try_new(TeamLife {
        id,
        owner,
        parent: parent.clone(),
        state: state.clone(),
        tasks: tasks.clone(),
        children: SpinLock::new(Vec::new()),
        life: Life::try_new().map_err(|_| NestError::OutOfMemory)?,
    })
    .map_err(|_| NestError::OutOfMemory)?;
    let team = Arc::try_new(Team {
        space,
        id,
        sire: sire.as_ref().map_or_else(TaskWeak::empty, |task| {
            TaskWeak::stored(Arc::downgrade(task), Origin::Sire)
        }),
        state,
        tasks: Tasks {
            members: tasks,
            held: SpinLock::new_level(Level::L3, Vec::new()),
        },
        life: life.clone(),
        operation: SpinLock::new(()),
    })
    .map_err(|_| NestError::OutOfMemory)?;
    if let Some(sire) = sire {
        let parent = &sire.ident.team.life;
        {
            let mut children = parent.children.lock();
            if children.len() >= MAX_CHILDREN {
                return Err(NestError::OutOfMemory);
            }
            children
                .try_reserve(1)
                .map_err(|_| NestError::OutOfMemory)?;
        }
        sire.heir
            .lock()
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        let _commit = commit();
        if sire.dead() || parent.closed() {
            return Err(NestError::NoRegion);
        }
        let mut depth = 1;
        let mut ancestor = parent.parent.as_deref();
        while let Some(node) = ancestor {
            depth += 1;
            ancestor = node.parent.as_deref();
        }
        if depth >= MAX_DEPTH {
            return Err(NestError::NoRegion);
        }
        let mut children = parent.children.lock();
        let mut heirs = sire.heir.lock();
        if children.len() >= MAX_CHILDREN || children.len() == children.capacity() {
            return Err(NestError::OutOfMemory);
        }
        if heirs.len() == heirs.capacity() {
            return Err(NestError::OutOfMemory);
        }
        children.push(life);
        heirs.push(team.clone());
    }
    Ok(team)
}
