use crate::system::control::{
    core::publication::Publications,
    serve::{
        self,
        material::Supplies,
        resource::Resources,
        start::{self, Images},
        unit::Control,
        watch::Watch,
    },
};
use crate::system::identity::serve::{install::Roster, names::Names};
use crate::system::operator::serve::install::Tree;
use crate::system::run::{bootstrap::Boot, cycle};
use crate::system::{boot, life};
use crate::unit::{Died, UnitFile};
use runtime::env::mail;

pub struct Fixture {
    pub control: Control,
    pub roster: Roster,
    pub tree: Tree,
    pub watch: Watch,
    pub publications: Publications,
    pub runtime: Resources,
    pub names: Names,
    pub images: Images,
    pub supplies: Supplies,
    pub activation: Option<crate::service::hub::bridge::Activation>,
}
impl Fixture {
    pub fn new(boot: Boot) -> Result<Self, ()> {
        let status = crate::system::boot::start()?;
        let entry =
            mail::unseal_hole(protocol::system::control::publication::ENTRY).map_err(|_| ())?;
        let mut fixture = Self {
            control: Control::new(status.clone()),
            roster: Roster::default(),
            tree: Tree::default(),
            watch: Watch::new()?,
            publications: Publications::new(),
            runtime: Resources::new(),
            names: Names::new(),
            images: Images {
                catalog: boot.catalog,
                entry,
            },
            supplies: Supplies::new(boot.machine, boot.accounts),
            activation: None,
        };
        if boot::install(
            &status,
            &mut fixture.roster,
            &mut fixture.tree,
            &mut fixture.publications,
            entry,
            &mut fixture.names,
            &mut fixture.watch,
        )
        .is_err()
        {
            let _ = runtime::env::room::doom(status.control);
            return Err(());
        }
        Ok(fixture)
    }
    pub fn assemble(&mut self, program: &UnitFile) -> Result<(), Died> {
        let service = start::stage(program, &mut self.control, &self.images, &self.roster)
            .map_err(|_| start::E_PROGRAM)?;
        if let Err(_) = super::identity::supply_to(self.roster.authority(), program, service.0) {
            self.control.discard(
                program.name(),
                service.0,
                &self.roster,
                &mut self.activation,
            );
            return Err(start::E_PROGRAM);
        }
        let machine = self.supplies.machine;
        start::finish(
            program,
            &mut self.control,
            service,
            &mut self.supplies,
            &self.roster,
            &mut self.activation,
            |control, activation| {
                cycle::poll(
                    control,
                    &self.roster,
                    &machine,
                    activation,
                    self.images.entry,
                    &mut self.publications,
                    &mut self.runtime,
                    &mut self.names,
                    &mut self.tree,
                )
            },
        )
        .map_err(|_| start::E_PROGRAM)
    }
    pub fn action(&mut self, name: &str, action: serve::lifecycle::Action) -> Result<Option<env::TaskId>, ()> {
        let mut plans = crate::system::run::schedule::lifecycle().map_err(|_| ())?;
        let mut operations = serve::lifecycle::Operations::new();
        operations.push(serve::lifecycle::Request { name: name.into(), action, back: None }).map_err(|_| ())?;
        loop {
            serve::driver::poll(&mut plans, &mut operations, &mut self.control,
                &self.roster, &mut self.supplies, &mut self.activation, &self.images).map_err(|_| ())?;
            self.progress().map_err(|_| ())?;
            let tracked = operations.0.front().ok_or(())?;
            if tracked.complete {
                return if tracked.operation.failure.is_some() { Err(()) } else { Ok(tracked.operation.task) };
            }
            runtime::env::room::sleep(core::time::Duration::from_millis(1)).map_err(|_| ())?;
        }
    }
    pub fn progress(&mut self) -> Result<(), &'static str> {
        cycle::poll(
            &self.control,
            &self.roster,
            &self.supplies.machine,
            &self.activation,
            self.images.entry,
            &mut self.publications,
            &mut self.runtime,
            &mut self.names,
            &mut self.tree,
        )
    }
    pub fn supervise(&mut self) -> Result<(), serve::Fail> {
        let result = serve::run(
            &mut self.watch,
            &mut self.control,
            &self.roster,
            &mut self.supplies,
            &mut self.activation,
            &self.images,
            &mut self.publications,
            &mut self.runtime,
            &mut self.names,
            &mut self.tree,
            &[],
        );
        if result.is_err() {
            let _ = runtime::env::room::doom(self.control.status.control);
        }
        result?;
        life::stop(&self.control.status)
    }
}
