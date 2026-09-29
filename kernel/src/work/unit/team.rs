use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::space::Space;

use super::task::{Task, TaskBuilder, TaskTag};
use super::weak::{Site, TaskWeak};

pub struct Team {
    pub(crate) space: Arc<Space>,
    pub(crate) tasks: SpinLock<Vec<TaskWeak>>,
    pub(crate) held: SpinLock<Vec<Arc<Task>>>,
    pub(crate) id: TeamId,
    pub(crate) sire: TaskWeak,
    default_entry: OnceLock<usize>,
}

impl Team {
    pub(crate) fn push_task(&self, task: &Arc<Task>) {
        self.tasks
            .lock()
            .push(TaskWeak::stored(Arc::downgrade(task), Site::TeamTasks));
    }

    pub(crate) fn prune_tasks(&self, exited: &Arc<Task>) {
        let exited_ptr = Arc::as_ptr(exited);
        self.tasks.lock().retain(|t| {
            if Weak::strong_count(t) == 0 {
                return false;
            }
            !(Weak::as_ptr(t) == exited_ptr)
        });
    }

    pub(crate) fn all_reaped(&self) -> bool {
        {
            let held = self.held.lock();
            if !held.is_empty() {
                return false;
            }
        }
        let g = self.tasks.lock();
        g.iter().all(|t| match t.upgrade() {
            Some(task) => {
                let reaped = task.tag() == TaskTag::Reaped;
                drop(task);
                reaped
            }
            None => true,
        })
    }

    pub(crate) fn tasks_snapshot(&self) -> Vec<TaskWeak> {
        let g = self.tasks.lock();
        let mut out: Vec<TaskWeak> = Vec::new();
        if out.try_reserve(g.len()).is_err() {
            return Vec::new();
        }
        out.extend(g.iter().map(|w| w.copy_at(Site::Snapshot)));
        out
    }

    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }

    pub(crate) fn hold(&self, task: &Arc<Task>) {
        self.held.lock().push(task.clone());
    }

    pub(crate) fn release_held(&self, task: &Arc<Task>) -> bool {
        let mut g = self.held.lock();
        match g.iter().position(|t| Arc::ptr_eq(t, task)) {
            Some(i) => {
                g.swap_remove(i);
                true
            }
            None => false,
        }
    }

    pub(crate) fn default_entry(&self) -> usize {
        self.default_entry.get().copied().unwrap_or(0)
    }

    pub(crate) fn set_default_entry(&self, va: usize) {
        let _ = self.default_entry.set(va);
    }

    pub(crate) fn sire(&self) -> Option<TaskId> {
        self.sire.upgrade().map(|t| t.ident.id)
    }
}

pub struct TeamBuilder {
    space: Space,
    sire: TaskWeak,
}

impl TeamBuilder {
    pub fn new(space: Space) -> TeamBuilder {
        TeamBuilder {
            space,
            sire: TaskWeak::empty(),
        }
    }

    pub fn sire(mut self, sire: TaskWeak) -> TeamBuilder {
        self.sire = sire;
        self
    }

    pub fn spawn(self) -> Result<Arc<Team>, crate::memory::manager::MapError> {
        let id = alloc_team_id();
        let team = crate::tag!(
            Team,
            Arc::new(Team {
                space: crate::tag!(Space, Arc::new(self.space)),
                tasks: SpinLock::new_level(Level::L3, Vec::new()),
                held: SpinLock::new_level(Level::L3, Vec::new()),
                id,
                sire: self.sire,
                default_entry: OnceLock::new(),
            })
        );
        if let Some(sire) = team.sire.upgrade() {
            sire.adopt(team.clone())
                .map_err(|()| crate::memory::manager::MapError::OutOfMemory)?;
        }
        Ok(team)
    }
}

pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

pub(crate) fn init_kernel(space: Arc<Space>) -> &'static Arc<Team> {
    KERNEL_TEAM.get_or_init(|| {
        let id = alloc_team_id();
        crate::tag!(
            Team,
            Arc::new(Team {
                space,
                tasks: SpinLock::new_level(Level::L3, Vec::new()),
                held: SpinLock::new_level(Level::L3, Vec::new()),
                id,
                sire: TaskWeak::empty(),
                default_entry: OnceLock::new(),
            })
        )
    })
}

pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}

static NEXT_TEAM_ID: AtomicUsize = AtomicUsize::new(1);

pub(crate) fn alloc_team_id() -> TeamId {
    TeamId::new(NEXT_TEAM_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitError {
    Load,
    Unreadable,
    OoM,
}
