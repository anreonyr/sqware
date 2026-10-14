use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::core::memory::manager::MapError;
use crate::core::work::commit;
use crate::core::work::unit::life::{Life, TeamLife};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::TaskWeak;
use crate::machine::sync::{Level, SpinLock};

use super::stage::TeamStage;
use super::state::TeamState;
use super::tasks::Tasks;
use super::{MAX_CHILDREN, MAX_DEPTH, Team};

static NEXT: AtomicUsize = AtomicUsize::new(1);

/// 下一个团队号：到 `isize::MAX` 就不再发新号（既有回绕口径）。
pub(crate) fn next_id() -> Result<TeamId, MapError> {
    let id = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            (id < isize::MAX as usize).then_some(id + 1)
        })
        .map_err(|_| MapError::OutOfMemory)?;
    Ok(TeamId::new(id))
}

/// 从零落一座团队：造状态机、成员表与生命周期线，再把自己的根挂到家长名下。
/// 家长位的深度与容量都在这里把关（失败时团队本身还没任何人看得到）。
pub(crate) fn make_team(
    space: Arc<Space>,
    sire: TaskWeak,
    parent: Option<Arc<TeamLife>>,
    owner: TaskId,
    constructing: bool,
) -> Result<Arc<Team>, MapError> {
    let id = next_id()?;
    let state = Arc::try_new(SpinLock::new_level(
        Level::UnitState,
        TeamState::new(if constructing {
            TeamStage::Constructing
        } else {
            TeamStage::Ready { default_entry: 0 }
        }),
    ))
    .map_err(|_| MapError::OutOfMemory)?;
    let tasks = Arc::try_new(SpinLock::new_level(Level::TeamTasks, Vec::new()))
        .map_err(|_| MapError::OutOfMemory)?;
    let life = Arc::try_new(TeamLife {
        id,
        owner,
        parent: parent.clone(),
        state: state.clone(),
        tasks: tasks.clone(),
        children: SpinLock::new(Vec::new()),
        life: Life::try_new().map_err(|_| MapError::OutOfMemory)?,
    })
    .map_err(|_| MapError::OutOfMemory)?;
    let team = Arc::try_new(Team {
        space,
        id,
        sire,
        state,
        tasks: Tasks {
            members: tasks,
            held: SpinLock::new_level(Level::L3, Vec::new()),
        },
        life: life.clone(),
        operation: SpinLock::new(()),
    })
    .map_err(|_| MapError::OutOfMemory)?;
    if let Some(parent) = parent {
        {
            let mut children = parent.children.lock();
            if children.len() >= MAX_CHILDREN {
                return Err(MapError::OutOfMemory);
            }
            children.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
        }
        let _commit = commit();
        let mut children = parent.children.lock();
        if parent.closed() {
            return Err(MapError::NoRegion);
        }
        let mut depth = 1;
        let mut ancestor = parent.parent.as_deref();
        while let Some(node) = ancestor {
            depth += 1;
            ancestor = node.parent.as_deref();
        }
        if depth >= MAX_DEPTH {
            return Err(MapError::NoRegion);
        }
        if children.len() >= MAX_CHILDREN || children.len() == children.capacity() {
            return Err(MapError::OutOfMemory);
        }
        children.push(life);
    }
    Ok(team)
}
