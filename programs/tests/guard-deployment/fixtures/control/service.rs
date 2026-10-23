use crate::system::control::unit::{
    table::{State, Table},
    task::Readiness,
    verdict::Fail,
};

pub struct Service {
    task: crate::TaskId,
}

impl Service {
    pub fn new(task: crate::TaskId) -> Self {
        Self { task }
    }

    pub fn task(&self) -> crate::TaskId {
        self.task
    }

    pub fn connect(&mut self, _: &crate::unit::UnitFile) -> Result<(), crate::Error> {
        crate::record("connect");
        Ok(())
    }

    pub fn ready(
        &mut self,
        table: &mut Table,
        readiness: Readiness<'_>,
    ) -> Result<bool, Fail> {
        crate::record("ready");
        table.set_state(readiness.name, State::Ready);
        Ok(true)
    }
}
