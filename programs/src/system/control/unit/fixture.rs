use super::{Control, start::Error};
use crate::system::control::unit::table::{Slot, State};
use crate::unit::UnitFile;
use env::TaskId;

impl Control {
    pub(crate) fn fixture_attach_unit(
        &mut self,
        program: &UnitFile,
        task: TaskId,
    ) -> Result<(), Error> {
        self.enlist(program)?;
        self.table
            .attach(program.name(), Slot::Live { task, team: None })
            .map_err(|_| Error::Table)?;
        self.table.set_state(program.name(), State::Ready);
        Ok(())
    }

    pub(crate) fn fixture_detach_unit(&mut self, name: &str) {
        self.table.detach(name);
        self.table.set_state(name, State::Dead);
    }
}
