//! Control owns this private installation face; only the live Hub can ask through it.
use env::{Access, PieToken, Policy, TaskId, Wait};
use protocol::communication::hand::Sender;
use protocol::communication::session::establish;
use protocol::service::hub::{self, activation::{self, Activate}, frame::Said};
use protocol::service::identity::CoalitionId;
use runtime::core::res::port;
use runtime::env::{mail::{self, HolePie}, unit};

use crate::system::control::{BOOT_MS, Control};
use env::wire::Span as _;

pub struct Activation {
    hub: TaskId,
    entry: PieToken,
}

impl Activation {
    pub fn open(hub: TaskId) -> Result<Self, &'static str> {
        let entry = mail::unseal_hole(activation::ENTRY).map_err(|_| "hub activation hole")?;
        let owned = Self { hub, entry };
        port::ship(&HolePie::from_token(entry), hub, Access::STORE, Policy::NONE)
            .map_err(|_| "hub activation injection")?;
        Ok(owned)
    }

    pub fn poll(&self, control: &Control) {
        let mut bytes = [0; runtime::PAGE_SIZE];
        while let Ok((len, from)) = HolePie::from_token(self.entry).pull(&mut bytes, Wait::POLL) {
            let Some(ask) = Activate::fetch_at(&bytes[..len], 0).map(|one| one.0) else {
                continue;
            };
            if !matches!(mail::reserve(ask.back), Ok((_, owner, mark))
                if owner == from && mark == activation::BACK)
            {
                continue;
            }
            let allowed = from == self.hub
                && control.task("hub") == Some(self.hub)
                && !unit::join(self.hub, Wait::POLL).unwrap_or(true)
                && control.tasks().any(|task| task == ask.task);
            let status = if allowed && control.roster.activate(ask.task, ask.coalition).is_ok() {
                hub::OK
            } else {
                hub::DENIED
            };
            let mut reply = Sender::<Said>::from_token(ask.back);
            let _ = reply.send(Said::of(status));
            drop(reply);
            let _ = mail::release(ask.back);
        }
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        let _ = mail::seal(self.entry);
        let _ = mail::release(self.entry);
    }
}

/// Ask the kernel-Sire-owned face injected at launch, not a matching public mark.
pub fn activate(task: TaskId, coalition: CoalitionId) -> Result<(), ()> {
    let sire = unit::sire();
    let entry = establish::find(sire, activation::ENTRY).ok_or(())?;
    if !matches!(mail::reserve(entry), Ok((vestor, owner, mark))
        if vestor == sire && owner == sire && mark == activation::ENTRY)
    {
        return Err(());
    }
    let (back, seed) = establish::lend_out(entry, activation::BACK)?;
    struct Back(PieToken);
    impl Drop for Back {
        fn drop(&mut self) {
            let _ = mail::seal(self.0);
            let _ = mail::release(self.0);
        }
    }
    let _back = Back(back);
    let mut request = Sender::<Activate>::from_token(entry);
    request.send_within(Activate { task, coalition, back: seed }, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())?;
    let mut bytes = [0; Said::LEN];
    let (n, from) = HolePie::from_token(back).pull(&mut bytes, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())?;
    let said = Said::fetch_at(&bytes[..n], 0).map(|one| one.0).ok_or(())?;
    (from == sire && said.status == hub::OK).then_some(()).ok_or(())
}
