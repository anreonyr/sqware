use crate::core::memory::nest::SpaceKind;

#[derive(Debug, Clone, Copy)]
pub struct WalkConfig {
    pub world: SpaceKind,
    pub gaps: bool,
    pub ceiling: usize,
}

impl WalkConfig {
    pub fn kernel(ceiling: usize) -> Self {
        Self {
            world: SpaceKind::Supervisor,
            gaps: false,
            ceiling,
        }
    }

    pub fn normal(world: SpaceKind, ceiling: usize) -> Self {
        Self {
            world,
            gaps: true,
            ceiling,
        }
    }
}
