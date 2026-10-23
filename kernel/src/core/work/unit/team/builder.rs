use alloc::sync::{Arc, Weak};

use crate::core::memory::nest::{NestError, SpaceKind};
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::core::work::unit::task::TaskWeak;

use super::Team;
use super::construct::make_team;

/// 造一座团队：空间给定，家长位可选。
pub struct TeamBuilder {
    space: Space,
    sire: TaskWeak,
    creator: env::TaskId,
    constructing: bool,
}

impl TeamBuilder {
    pub fn new(space: Space) -> Self {
        Self {
            space,
            sire: TaskWeak::empty(),
            creator: env::TaskId::new(0),
            constructing: false,
        }
    }

    pub fn sire(mut self, sire: TaskWeak) -> Self {
        self.sire = sire;
        self
    }

    pub fn creator(mut self, creator: env::TaskId) -> Self {
        self.creator = creator;
        self
    }

    pub(crate) fn constructing(mut self) -> Self {
        self.constructing = true;
        self
    }

    /// 按 `kind` 起一个空间（非内核的会带跳板），再以 `sire` 落一座构造中的团队。
    pub(crate) fn build(kind: SpaceKind, sire: TaskWeak) -> Result<Arc<Team>, NestError> {
        Self::build_with_creator(kind, sire, env::TaskId::new(0))
    }

    pub(crate) fn build_with_creator(
        kind: SpaceKind,
        sire: TaskWeak,
        creator: env::TaskId,
    ) -> Result<Arc<Team>, NestError> {
        let space = match kind {
            SpaceKind::Supervisor => SpaceBuilder::supervisor(),
            SpaceKind::User => SpaceBuilder::user(),
        }
        .build()?;
        TeamBuilder::new(space).sire(sire).creator(creator).constructing().spawn()
    }

    pub fn spawn(self) -> Result<Arc<Team>, NestError> {
        let sire = match self.sire.upgrade() {
            Some(task) => Some(task),
            None if self.sire.as_weak().ptr_eq(&Weak::new()) => None,
            None => return Err(NestError::NoRegion),
        };
        let space = Arc::try_new(self.space).map_err(|_| NestError::OutOfMemory)?;
        make_team(space, sire, self.creator, self.constructing)
    }
}
