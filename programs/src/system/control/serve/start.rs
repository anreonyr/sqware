use super::material::Supplies;
use super::task as service;
use super::unit::{Control, Pending, Service};
use crate::boot::Catalog;
use crate::service::hub::bridge::Activation;
use crate::system::control::core::unit::{Slot, State};
use crate::system::control::core::verdict::Fail;
use crate::system::identity::serve::install::Roster;
use crate::system::run::source::Source;
use crate::unit::{Died, PROGRAMS};
use crate::unit::{Setup, UnitFile};
use ::core::time::Duration;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use env::{Mark, TaskId, Wait};
use protocol::communication::session::establish::{self, Endpoint};
pub const RETRY_MS: usize = 1;

pub const READY_MS: usize = 1000;

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
    pub fn mint(&mut self, name: String, images: &Images) -> Result<(), Fail> {
        let program = program_of(name.as_str()).ok_or(Fail::Unknown)?;
        let method = program.name();
        match self.table.find(method) {
            Some(row) => {
                if !matches!(row.state, State::NeverStarted | State::Dead) {
                    return Err(Fail::NotReady);
                }
            }
            None => {
                self.enlist(program).map_err(|_| Fail::Unknown)?;
            }
        }
        self.pending.try_reserve(1).map_err(|_| Fail::Full)?;
        let service = match self.spawn(program, images) {
            Ok(service) => service,
            Err(Error::Missing | Error::Step(_)) => return Err(Fail::BadImage),
            Err(Error::Spawn) => return Err(Fail::Full),
            Err(_) => return Err(Fail::Unknown),
        };
        if super::publication::inject(images.entry, service.0).is_err() {
            self.discard_image(program.name(), service.0);
            return Err(Fail::BadImage);
        }
        self.pending.push(Pending {
            name: program.name(),
            service,
        });
        Ok(())
    }
    pub fn embark(
        &mut self,
        name: String,
        requester: TaskId,
        roster: &Roster,
        activation: &mut Option<Activation>,
        supplies: &mut Supplies,
        mut progress: impl FnMut(&Control, &Option<Activation>) -> Result<(), &'static str>,
    ) -> Result<Service, Fail> {
        if self.table.find(&name).is_some_and(|r| r.state == State::Debarked) { return self.resume(&name); }
        let at = self
            .pending
            .iter()
            .position(|p| p.name == name.as_str())
            .ok_or(Fail::NotReady)?;
        let mut pending = self.pending.remove(at);
        let program = program_of(pending.name).ok_or(Fail::Unknown)?;
        let released = (|| {
            roster
                .inherit(pending.service.0, requester)
                .map_err(|_| Fail::NotReady)?;
            progress(self, activation).map_err(|_| Fail::NotReady)?;
            connect_all(program, &mut pending.service).map_err(|_| Fail::NotReady)?;
            let method = pending.name.to_string();
            self.launch(
                program,
                method.clone(),
                &mut pending.service,
                supplies,
                activation,
            )
            .map_err(|_| Fail::NotReady)?;
            self.ready(
                method,
                &mut pending.service,
                program.supply(),
                activation,
                &mut progress,
            )
            .map_err(|_| Fail::NotReady)
        })();
        if let Err(fail) = released {
            self.discard(pending.name, pending.service.0, roster, activation);
            let _ = progress(self, activation);
            return Err(fail);
        }
        Ok(pending.service)
    }
    pub fn spawn(&mut self, program: &UnitFile, images: &Images) -> Result<Service, Error> {
        let name = program.name().to_string();
        let entry = images.catalog.find(name.as_str()).ok_or(Error::Missing)?;
        let Some(image) = Source::initrd(images.catalog).image(name.clone()) else {
            return Err(Error::Missing);
        };
        let task = service::mint(&mut self.table, name.as_str(), image, entry.kind)
            .map_err(|_| Error::Spawn)?;
        Ok((task, Vec::new()))
    }
    pub fn activate(&mut self, name: &str, service: &mut Service) -> Result<(), Error> {
        let (task, channels) = service;
        service::embark(
            &mut self.table,
            name,
            *task,
            &[],
            channels.as_mut_slice(),
            &[],
            Wait::POLL,
        )
        .map_err(|fail| {
            let why = match fail {
                Fail::Unknown => "unknown",
                Fail::BadImage => "bad image",
                Fail::Full => "full",
                Fail::NotReady => "not ready",
            };
            protocol::debug::put(&alloc::format!("system: start failed {name} why={why}"));
            Error::Step("start failed")
        })
    }
    pub fn ready(
        &mut self,
        name: String,
        service: &mut Service,
        setup: &'static [Setup],
        activation: &Option<Activation>,
        mut progress: impl FnMut(&Control, &Option<Activation>) -> Result<(), &'static str>,
    ) -> Result<(), Error> {
        let mut marks: Vec<Mark> = Vec::new();
        for s in setup {
            for ch in [Some(s.channel()), s.ready()].into_iter().flatten() {
                marks
                    .try_reserve(1)
                    .map_err(|_| Error::Step("no room for marks"))?;
                marks.push(Mark::of(ch));
            }
        }
        let until = runtime::env::chrono::clock().saturating_add(BOOT_MS as u64 * 1_000_000);
        let fail = loop {
            progress(self, activation).map_err(Error::Step)?;
            match service::ready(
                &mut self.table,
                name.as_str(),
                service.1.as_mut_slice(),
                &marks,
                Wait::POLL,
            ) {
                Ok(already)
                    if already
                        || self
                            .table
                            .find(name.as_str())
                            .is_some_and(|row| row.state == State::Ready) =>
                {
                    return Ok(());
                }
                Ok(_) if runtime::env::chrono::clock() < until => {
                    runtime::env::room::sleep(Duration::from_millis(RETRY_MS as u64))
                        .map_err(|_| Error::Step("ready wait"))?;
                }
                Ok(_) => break Fail::NotReady,
                Err(fail) => break fail,
            }
        };
        let why = match fail {
            Fail::Unknown => "unknown",
            Fail::BadImage => "bad image",
            Fail::Full => "full",
            Fail::NotReady => "not ready",
        };
        protocol::debug::put(&alloc::format!(
            "system: not ready {name} why={why} marks={}",
            marks.len(),
        ));
        Err(Error::Step("start failed"))
    }
    pub fn launch(
        &mut self,
        program: &UnitFile,
        name: String,
        service: &mut Service,
        supplies: &mut Supplies,
        activation: &mut Option<Activation>,
    ) -> Result<(), Error> {
        if program.name() == "hub" {
            *activation = Some(
                crate::service::hub::bridge::Activation::open(service.0).map_err(Error::Step)?,
            );
        }
        self.activate(name.as_str(), service)?;
        if let Some(load) = program
            .demand
            .supply
            .iter()
            .find(|s| s.machine())
            .map(Setup::channel)
        {
            supplies.enroll(name, service, load)?;
        }
        Ok(())
    }
    fn discard_image(&mut self, name: &str, task: TaskId) {
        let _ = runtime::env::room::doom(task);
        if let Some(row) = self.table.find(name)
            && let Slot::Live {
                team: Some(team), ..
            } = row.slot
        {
            let _ = runtime::env::unit::oust(team);
        }
        self.table.detach(name);
        self.table.set_state(name, State::Dead);
    }
}
pub fn connect_all(program: &UnitFile, service: &mut Service) -> Result<(), Error> {
    for s in program.supply() {
        for ch in [Some(s.channel()), s.ready()].into_iter().flatten() {
            service
                .1
                .try_reserve(1)
                .map_err(|_| Error::Step("no room for channels"))?;
            let channel = connect(service.0, ch)?;
            service.1.push(channel);
        }
    }
    Ok(())
}
pub fn connect(to: TaskId, ch: &'static str) -> Result<Endpoint, Error> {
    establish::endpoint(to, Mark::of(ch), Wait::POLL).map_err(|_| Error::Step("connect failed"))
}
pub(crate) fn program_of(name: &str) -> Option<&'static UnitFile> {
    PROGRAMS
        .iter()
        .copied()
        .find(|p| p.relation.after.is_some() && p.name() == name)
}

pub fn stage(
    program: &UnitFile,
    control: &mut Control,
    images: &Images,
    roster: &Roster,
) -> Result<Service, Error> {
    for dep in program.relation.after.unwrap_or(&[]) {
        if !crate::unit::is_target(dep) {
            control
                .await_ready(dep, Wait::AtMost(READY_MS))
                .map_err(|_| Error::Step("dep not ready"))?;
        }
    }
    control.enlist(program)?;
    let service = control.spawn(program, images)?;
    if let Err(why) = super::publication::inject(images.entry, service.0) {
        control.discard_image(program.name(), service.0); return Err(Error::Step(why));
    }
    if let Err(why) = roster.authorize(service.0) {
        control.discard_image(program.name(), service.0);
        let _ = roster.unbind(service.0);
        return Err(Error::Step(why));
    }
    control.table.mark_static(service.0);
    Ok(service)
}

pub fn finish(
    program: &UnitFile,
    control: &mut Control,
    mut service: Service,
    supplies: &mut Supplies,
    roster: &Roster,
    activation: &mut Option<Activation>,
    mut progress: impl FnMut(&Control, &Option<Activation>) -> Result<(), &'static str>,
) -> Result<(), Error> {
    let result = (|| {
        connect_all(program, &mut service)?;
        progress(control, activation).map_err(Error::Step)?;
        control.launch(
            program,
            program.name().to_string(),
            &mut service,
            supplies,
            activation,
        )?;
        control.ready(
            program.name().to_string(),
            &mut service,
            program.supply(),
            activation,
            &mut progress,
        )
    })();
    if result.is_err() {
        control.discard(program.name(), service.0, roster, activation);
        let _ = progress(control, activation);
    }
    result
}
