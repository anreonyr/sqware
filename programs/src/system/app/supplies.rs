use crate::boot::Accounts;
use crate::support::machine::Machine;
use crate::system::control::unit::Service;
use crate::system::control::unit::start::{BOOT_MS, Error};
use ::resource::port;
use env::{Access, Entry, Mark, Name, PieKind, Policy, Wait};
use hub_api::{ENROLL_MAX, Enroll};
use programs::debug;

pub struct Supplies {
    pub machine: Machine,
    catalog: crate::boot::Catalog<'static>,
    pub(super) accounts: Accounts,
    pub(super) out: ipc::hand::Sender<hub_api::Enroll>,
}
impl Supplies {
    pub fn new(
        machine: Machine,
        accounts: Accounts,
        catalog: crate::boot::Catalog<'static>,
    ) -> Self {
        Self {
            machine,
            catalog,
            accounts,
            out: ipc::hand::Sender::new(),
        }
    }
}
impl Supplies {
    pub fn image(
        &mut self,
        service: &mut Service,
        (name, load): (&str, &str),
    ) -> Result<(), Error> {
        use wire::Message;
        let bytes = self
            .catalog
            .find(name)
            .ok_or(Error::Step("image supply missing"))?
            .elf;
        let tx = service.claim_supply(Mark::of(load), Wait::AtMost(BOOT_MS))?;
        let size = bytes
            .len()
            .checked_add(env::PAGE_SIZE - 1)
            .map(|size| size / env::PAGE_SIZE * env::PAGE_SIZE)
            .filter(|size| *size != 0)
            .ok_or(Error::Step("image supply size"))?;
        let root = env::pie::unseal_pole(size, true)
            .map_err(|_| Error::Step("image supply allocation"))?;
        let sent = (|| {
            let (base, size) =
                resource::raw::open(root).map_err(|_| Error::Step("image supply mapping"))?;
            if size < bytes.len() {
                return Err(Error::Step("image supply mapping short"));
            }
            // SAFETY: the newly allocated writable mapping covers the catalog payload.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), base as *mut u8, bytes.len());
            }
            env::pie::shut(root).map_err(|_| Error::Step("image supply unmap"))?;
            let sent = env::pie::accord(
                root,
                service.task(),
                env::Permission::FETCH,
                crate::unit::IMAGE_MARK,
            )
            .map_err(|_| Error::Step("image supply grant"))?;
            let frame = crate::unit::ImageSupplyFrame {
                seed: sent,
                length: bytes.len() as u64,
            };
            let mut buffer = crate::unit::ImageSupplyFrame::EMPTY;
            let length = frame
                .store(&mut buffer)
                .ok_or(Error::Step("image supply frame"))?;
            resource::port::Sender::import(tx)
                .map_err(|_| Error::Step("image supply channel"))?
                .push(&buffer[..length], Wait::AtMost(BOOT_MS))
                .map_err(|_| Error::Step("image supply send"))?;
            Ok(())
        })();
        if let Err(error) = sent {
            let _ = env::pie::shut(root);
            let _ = env::pie::release(root);
            return Err(error);
        }
        service.hold_supply(root)
    }
    pub fn grant_call(&self, call: env::Call, task: env::TaskId) -> Result<(), &'static str> {
        let token = self
            .accounts
            .token(Name::Call(call))
            .ok_or("Call authority missing")?;
        env::pie::accord(token, task, env::Permission::FETCH, Mark::NONE)
            .map_err(|_| "Call authority grant")?;
        Ok(())
    }
    pub fn enroll(
        &mut self,
        service: &mut Service,
        program: &crate::unit::UnitFile,
    ) -> Result<(), Error> {
        let task = service.task();
        let name = program.name();
        let load = program
            .supply()
            .iter()
            .find(|setup| setup.machine())
            .map(crate::unit::Setup::channel)
            .ok_or(Error::Step("no machine supply"))?;
        let tx = service.claim_supply(Mark::of(load), Wait::AtMost(BOOT_MS))?;

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
                PieKind::Pole => port::ship(src, task, access, policy).ok(),
                PieKind::Nole => port::ship(src, task, access, policy).ok(),
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
        put(
            Name::Page(env::Page::Dtb),
            PieKind::Pole,
            Access::FETCH,
            Policy::VEST,
        );
        put(
            Name::Trap(env::Trap::SupervisorExternal),
            PieKind::Nole,
            Access::FETCH,
            Policy::VEST,
        );
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
