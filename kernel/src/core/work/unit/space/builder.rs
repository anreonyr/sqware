use crate::core::memory::manager::{MapError, Memory, SpaceKind};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::team;

use super::space::Space;

/// 建一个空间。
pub struct SpaceBuilder {
    kind: SpaceKind,
    kernel: bool,
}

impl SpaceBuilder {
    pub fn kernel() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            kernel: true,
        }
    }

    pub fn supervisor() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            kernel: false,
        }
    }

    pub fn user() -> Self {
        Self {
            kind: SpaceKind::User,
            kernel: false,
        }
    }

    pub fn build(self) -> Result<Space, MapError> {
        let memory = Memory::new(self.kind, self.kernel)?;
        let space = Space {
            memory,
            life: Life::try_new()?,
        };
        if !self.kernel {
            let source = team::kernel().expect("kernel team not initialized");
            space.memory.seed_trampoline(&source.space.memory)?;
        }
        Ok(space)
    }
}
