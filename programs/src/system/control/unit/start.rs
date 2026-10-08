use super::{Control, Service, task as service};
pub use crate::support::timing::BOOT_MS;
use crate::system::control::unit::task::Image;
use crate::{
    boot::Catalog,
    unit::{Died, PROGRAMS, UnitFile},
};
use alloc::vec::Vec;
use env::{Mark, Wait};
use ipc::session::establish;

pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Missing,
    Table,
    Spawn,
    Step(&'static str),
}

pub struct Images {
    pub catalog: Catalog<'static>,
    pub entry: env::PieToken,
}
impl Images {
    pub fn inject(&self, task: env::TaskId) -> Result<(), &'static str> {
        ::resource::port::ship(self.entry, task, env::Access::STORE, env::Policy::NONE)
            .map(|_| ())
            .map_err(|_| "publication inject")
    }
}
impl Control {
    pub fn spawn(&mut self, program: &UnitFile, images: &Images) -> Result<Service, Error> {
        let name = program.name();
        let entry = images.catalog.find(name).ok_or(Error::Missing)?;
        let task = service::mint(
            &mut self.table,
            &mut self.loader,
            Image {
                name,
                bytes: entry.elf,
                kind: entry.kind,
            },
        )
        .map_err(|_| Error::Spawn)?;
        Ok((task, Vec::new()))
    }
}
pub fn connect_all(program: &UnitFile, service: &mut Service) -> Result<(), Error> {
    for s in program.supply() {
        for ch in [Some(s.channel()), s.ready()].into_iter().flatten() {
            service
                .1
                .try_reserve(1)
                .map_err(|_| Error::Step("no room for channels"))?;
            let channel = establish::endpoint(service.0, Mark::of(ch), Wait::POLL)
                .map_err(|_| Error::Step("connect failed"))?;
            service.1.push(channel);
        }
    }
    Ok(())
}
pub(crate) fn program_of(
    name: &str,
) -> Result<&'static UnitFile, crate::system::control::unit::verdict::Fail> {
    PROGRAMS
        .iter()
        .copied()
        .find(|p| p.relation.after.is_some() && p.name() == name)
        .ok_or(crate::system::control::unit::verdict::Fail::Unknown)
}
