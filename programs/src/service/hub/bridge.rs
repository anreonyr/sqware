//! Control owns this private installation face; only the live Hub can ask through it.
use env::{Access, PieToken, Policy, TaskId, Wait};
use protocol::common::schedule::{Progress, Res};
use protocol::communication::hand::Sender;
use protocol::communication::session::establish;
use protocol::service::hub::{
    self,
    activation::{self, Activate},
    frame::Said,
};
use protocol::system::identity::CoalitionId;
use runtime::core::res::port;
use env::unit;

use crate::system::control::serve::start::BOOT_MS;
use crate::system::control::serve::unit::Control;
use env::wire::Span as _;
use env::pie;
use runtime::core::res::pie::{HolePie, reserve};

pub struct Activation {
    hub: TaskId,
    entry: PieToken,
}

impl Activation {
    pub(crate) fn entry(&self) -> PieToken {
        self.entry
    }

    pub fn open(hub: TaskId) -> Result<Self, &'static str> {
        let entry = pie::unseal_hole(activation::ENTRY).map_err(|_| "hub activation hole")?;
        let owned = Self { hub, entry };
        port::ship(
            &HolePie::from_token(entry),
            hub,
            Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| "hub activation injection")?;
        Ok(owned)
    }

    pub fn poll(
        &self,
        control: &Control,
        roster: &crate::system::identity::serve::install::Roster,
    ) {
        let mut bytes = [0; runtime::PAGE_SIZE];
        while let Ok((len, from)) = HolePie::from_token(self.entry).pull(&mut bytes, Wait::POLL) {
            let Some(ask) = Activate::fetch_at(&bytes[..len], 0).map(|one| one.0) else {
                continue;
            };
            if !matches!(reserve(ask.back), Ok((_, owner, mark))
                if owner == from && mark == activation::BACK)
            {
                continue;
            }
            let allowed = from == self.hub
                && control.task("hub") == Some(self.hub)
                && !unit::join(self.hub, Wait::POLL).unwrap_or(true)
                && control.tasks().any(|task| task == ask.task);
            // **一趟里的每一枚都要落**：有一枚不成 ⇒ 整趟答 `DENIED`（起手那一侧据此当场收手）
            let status = if allowed
                && (0..ask.len()).all(|i| {
                    ask.coalition(i)
                        .is_some_and(|c| roster.activate(ask.task, c).is_ok())
                }) {
                hub::OK
            } else {
                hub::DENIED
            };
            let mut reply = Sender::<Said>::from_token(ask.back);
            let _ = reply.send(Said::of(status));
            drop(reply);
            let _ = pie::release(ask.back);
        }
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        let _ = pie::seal(self.entry);
        let _ = pie::release(self.entry);
    }
}

/// Ask the kernel-Sire-owned face injected at launch, not a matching public mark.
/// **一趟报一批**（见 [`activation::Activate`]）：枚数写进帧里，`BACK` 那一条回话只有一个状态。
pub fn activate(task: TaskId, coalitions: &[CoalitionId]) -> Result<(), ()> {
    let sire = unit::sire();
    let entry = establish::find(sire, activation::ENTRY).ok_or(())?;
    if !matches!(reserve(entry), Ok((vestor, owner, mark))
        if vestor == sire && owner == sire && mark == activation::ENTRY)
    {
        return Err(());
    }
    let (back, seed) = establish::lend_out(entry, activation::BACK)?;
    struct Back(PieToken);
    impl Drop for Back {
        fn drop(&mut self) {
            let _ = pie::seal(self.0);
            let _ = pie::release(self.0);
        }
    }
    let _back = Back(back);
    let frame = Activate::of(task, coalitions, seed).ok_or(())?;
    let mut request = Sender::<Activate>::from_token(entry);
    request
        .send_within(frame, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())?;
    let mut bytes = [0; Said::LEN];
    let (n, from) = HolePie::from_token(back)
        .pull(&mut bytes, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())?;
    let said = Said::fetch_at(&bytes[..n], 0).map(|one| one.0).ok_or(())?;
    (from == sire && said.status == hub::OK)
        .then_some(())
        .ok_or(())
}

pub(crate) fn maintain(
    activation: Res<Option<Activation>>,
    control: Res<crate::system::control::serve::unit::Control>,
    roster: Res<crate::system::identity::serve::install::Roster>,
) -> Result<Progress, &'static str> {
    if let Some(activation) = activation.as_ref() {
        activation.poll(&control, &roster);
    }
    Ok(Progress::Done)
}
