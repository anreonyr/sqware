use super::start::{BOOT_MS, Error};
use super::unit::Service;
use crate::boot::Accounts;
use crate::system::common::machine::Machine;
use env::{Access, Name, Mark, Entry, PieKind, Policy, Wait};
use programs::debug;
use hub_api::{ENROLL_MAX, Enroll};
use ::resource::port;

pub struct Supplies {
    pub machine: Machine,
    pub(super) accounts: Accounts,
    pub(super) out: ipc::hand::Sender<hub_api::Enroll>,
}
impl Supplies {
    pub fn new(machine: Machine, accounts: Accounts) -> Self {
        Self {
            machine,
            accounts,
            out: ipc::hand::Sender::new(),
        }
    }
}
impl Supplies {
    pub fn grant_call(&self, call: env::Call, task: env::TaskId) -> Result<(), &'static str> {
        let token = self.accounts.token(Name::Call(call)).ok_or("Call authority missing")?;
        env::pie::accord(token, task, env::Permission::FETCH, Mark::NONE)
            .map_err(|_| "Call authority grant")?;
        Ok(())
    }
    pub fn enroll(
        &mut self,
        service: &mut Service,
        program: &crate::unit::UnitFile,
    ) -> Result<(), Error> {
        let (task, channels) = service;
        let name = program.name();
        let load = program
            .supply()
            .iter()
            .find(|setup| setup.machine())
            .map(crate::unit::Setup::channel)
            .ok_or(Error::Step("no machine supply"))?;
        let Some(link) = channels.first_mut() else {
            return Err(Error::Step("no channel"));
        };
        if !link.claim(*task, Mark::of(load), Wait::AtMost(BOOT_MS)) {
            return Err(Error::Step("no channel"));
        }
        let Some(tx) = link.tx() else {
            return Err(Error::Step("no channel"));
        };

        let devices = self
            .machine
            .devices()
            .ok_or(Error::Step("no room for devices"))?;
        let total = devices.len() + 2;
        if total > ENROLL_MAX {
            return Err(Error::Step("too many devices"));
        }

        let mut records = [Entry::NONE; ENROLL_MAX];
        let mut got = 0usize;
        let mut put = |key: Name, kind: PieKind, access: Access, policy: Policy| {
            let shipped = self.accounts.token(key).and_then(|src| match kind {
                PieKind::Pole => port::ship(src, *task, access, policy).ok(),
                PieKind::Nole => port::ship(src, *task, access, policy).ok(),
                PieKind::Hole | PieKind::Tole => None,
            });
            match shipped {
                Some(seat) => {
                    records[got] = Entry::new(key, kind, seat.seed());
                    got += 1;
                }
                None => debug!(
                    "system: enroll {} skipped {:#x}",
                    name,
                    key.base().unwrap_or(0)
                ),
            }
        };
        put(Name::Page(env::Page::Dtb), PieKind::Pole, Access::FETCH, Policy::VEST);
        put(Name::Trap(env::Trap::SupervisorExternal), PieKind::Nole, Access::FETCH, Policy::VEST);
        for device in &devices {
            put(
                device.resource,
                PieKind::Pole,
                Access::FETCH_STORE,
                Policy::VEST | Policy::ONLY,
            );
        }

        let Some(enroll) = Enroll::of(&records[..got]) else {
            return Err(Error::Step("too many devices"));
        };
        self.out = ipc::hand::Sender::<Enroll>::from_raw(tx);
        if self.out.send(enroll).is_err() {
            return Err(Error::Step("no channel"));
        }
        debug!("system: enrolled {} supplies for {}", got, name);
        Ok(())
    }
}
