use super::start::{BOOT_MS, Error};
use super::unit::Service;
use crate::boot::Accounts;
use crate::system::common::machine::Machine;
use env::{Access, Key, Mark, Pair, PieKind, Policy, Wait};
use protocol::debug;
use protocol::service::hub::{ENROLL_MAX, Enroll};
use runtime::core::res::port;
use runtime::env::mail::{NolePie, PolePie};
pub struct Supplies {
    pub machine: Machine,
    pub(super) accounts: Accounts,
    pub(super) out: protocol::communication::hand::Sender<protocol::service::hub::Enroll>,
}
impl Supplies {
    pub fn new(machine: Machine, accounts: Accounts) -> Self {
        Self {
            machine,
            accounts,
            out: protocol::communication::hand::Sender::new(),
        }
    }
}
impl Supplies {
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

        let mut records = [Pair::NONE; ENROLL_MAX];
        let mut got = 0usize;
        let mut put = |key: Key, kind: PieKind, access: Access, policy: Policy| {
            let shipped = self.accounts.token(key).and_then(|src| match kind {
                PieKind::Pole => port::ship(&PolePie::from_token(src), *task, access, policy).ok(),
                PieKind::Nole => port::ship(&NolePie::from_token(src), *task, access, policy).ok(),
            });
            match shipped {
                Some(seat) => {
                    records[got] = Pair::new(key, seat.seed());
                    got += 1;
                }
                None => debug!(
                    "system: enroll {} skipped {:#x}",
                    name,
                    key.base().unwrap_or(0)
                ),
            }
        };
        put(Key::dtb(), PieKind::Pole, Access::FETCH, Policy::VEST);
        put(Key::irq(), PieKind::Nole, Access::FETCH, Policy::VEST);
        for device in &devices {
            put(
                device.key,
                PieKind::Pole,
                Access::FETCH_STORE,
                Policy::VEST | Policy::ONLY,
            );
        }

        let Some(enroll) = Enroll::of(&records[..got]) else {
            return Err(Error::Step("too many devices"));
        };
        self.out = protocol::communication::hand::Sender::<Enroll>::from_token(tx);
        if self.out.send(enroll).is_err() {
            return Err(Error::Step("no channel"));
        }
        debug!("system: enrolled {} supplies for {}", got, name);
        Ok(())
    }
}
