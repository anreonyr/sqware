use super::Control;
use super::table::{Slot, State};

impl Control {
    pub(crate) fn reclaimed_at(&self) -> u64 {
        self.reclaimed_at
    }

    pub(crate) fn detach_unit(
        &mut self,
        name: &str,
    ) -> Result<(), super::verdict::Fail> {
        let occupied = self
            .table
            .find(name)
            .is_some_and(|row| matches!(row.slot, Slot::Live { .. }));
        self.table.detach(name)?;
        self.table.set_state(name, State::Dead);
        if occupied {
            self.reclaimed_at = env::chrono::clock();
        }
        Ok(())
    }
}
