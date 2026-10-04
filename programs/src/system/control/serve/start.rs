use super::{
    task as service,
    unit::{Control, Service},
};
use crate::system::control::serve::task::Image;
use crate::system::run::source::Source;
use crate::{
    boot::Catalog,
    unit::{Died, PROGRAMS, UnitFile},
};
use alloc::{string::ToString, vec::Vec};
use env::{Mark, Wait};
use protocol::communication::session::establish;
pub const RETRY_MS: usize = 1;
pub const BOOT_MS: usize = 5000;

pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Manifest,
    Missing,
    Table,
    Spawn,
    Step(&'static str),
}

impl Error {
    pub fn said(self) -> &'static str {
        match self {
            Error::Manifest => "bad name",
            Error::Missing => "not in catalog",
            Error::Table => "no table row",
            Error::Spawn => "spawn failed",
            Error::Step(what) => what,
        }
    }
}

pub struct Images {
    pub catalog: Catalog<'static>,
    pub entry: env::PieToken,
}
impl Control {
    pub fn spawn(&mut self, program: &UnitFile, images: &Images) -> Result<Service, Error> {
        let name = program.name().to_string();
        let entry = images.catalog.find(name.as_str()).ok_or(Error::Missing)?;
        let Some(image) = Source::initrd(images.catalog).image(name.clone()) else {
            return Err(Error::Missing);
        };
        let task = service::mint(
            &mut self.table,
            Image {
                name: name.as_str(),
                bytes: image,
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
) -> Result<&'static UnitFile, crate::system::control::core::verdict::Fail> {
    PROGRAMS
        .iter()
        .copied()
        .find(|p| p.relation.after.is_some() && p.name() == name)
        .ok_or(crate::system::control::core::verdict::Fail::Unknown)
}
