use super::{Control, Service, task as service};
pub use crate::support::timing::BOOT_MS;
use crate::unit::{Died, UnitFile};

pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Table,
    Spawn,
    Step(&'static str),
}
#[derive(Clone, Copy)]
pub(crate) struct Input {
    pub program: &'static UnitFile,
    pub image: Option<(&'static [u8], env::ProgramKind)>,
}
impl Control {
    pub(crate) fn input(&self, name: &str) -> Result<Input, super::verdict::Fail> {
        self.inputs
            .iter()
            .copied()
            .find(|input| input.program.name() == name)
            .ok_or(super::verdict::Fail::Unknown)
    }
    pub(crate) fn attach_service(
        &mut self,
        name: &str,
        built: system_api::loader::Built,
    ) -> Result<Service, Error> {
        service::mint(&mut self.table, name, built)
            .map(Service::new)
            .map_err(|_| Error::Spawn)
    }
}
