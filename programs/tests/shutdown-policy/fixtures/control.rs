use super::instance::Instance;
use crate::system::control::unit::{table::Table, verdict};

pub struct Control {
    pub table: Table,
    pub instances: Vec<Instance>,
    pub reclaimed_at: u64,
}

impl Control {
    pub fn living_count(&self) -> usize {
        self.table.living().count()
    }

    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter()
    }

    pub fn walking(&self) -> bool {
        verdict::walking(&self.table)
    }
}
