//! Control owns this private installation face; only the live Hub can ask through it.
use ::resource::port;
use ::schedule::{Progress, Res};
use env::unit;
use env::{Access, PieToken, Policy, TaskId, Wait};
use hub_api::{
    self as hub,
    activation::{self, Activate},
    frame::Said,
};
use ipc::hand::Sender;

use crate::system::control::unit::Control;
use ::resource::raw::{Hole, reserve};
use env::pie;
use env::wire::Span as _;

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
        port::ship(entry, hub, Access::STORE, Policy::NONE)
            .map_err(|_| "hub activation injection")?;
        Ok(owned)
    }

    pub(crate) fn poll(
        &self,
        control: &Control,
        roster: &crate::system::control::identity::Roster,
    ) {
        let mut bytes = [0; env::PAGE_SIZE];
        while let Ok((len, from)) = Hole::from_raw(self.entry).pull(&mut bytes, Wait::POLL) {
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
            let mut reply = Sender::<Said>::from_raw(ask.back);
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

pub(crate) fn maintain(
    activation: Res<Option<Activation>>,
    control: Res<crate::system::control::unit::Control>,
    roster: Res<crate::system::control::identity::Roster>,
) -> Result<Progress, &'static str> {
    if let Some(activation) = activation.as_ref() {
        activation.poll(&control, &roster);
    }
    Ok(Progress::Done)
}

pub(crate) fn hooks() -> Result<crate::system::control::ActivationHooks, ::schedule::BuildError> {
    let mut prepare = ::schedule::Schedule::sequence();
    prepare.system("open", open)?;
    let mut retire = ::schedule::Schedule::sequence();
    retire.system("close", close)?;
    Ok(crate::system::control::ActivationHooks {
        prepare: prepare.build()?,
        retire: retire.build()?,
    })
}
fn open(
    active: Res<crate::system::control::lifecycle::Active>,
    mut activation: ::schedule::ResMut<Option<Activation>>,
) -> Result<Progress, crate::system::control::unit::verdict::Fail> {
    use crate::system::control::unit::verdict::Fail;
    if active.is_launching(hub_api::NAME)? {
        *activation = Some(
            Activation::open(active.task().ok_or(Fail::Unknown)?).map_err(|_| Fail::NotReady)?,
        );
    }
    Ok(Progress::Done)
}
fn close(
    active: Res<crate::system::control::lifecycle::Active>,
    mut activation: ::schedule::ResMut<Option<Activation>>,
) -> Result<Progress, crate::system::control::unit::verdict::Fail> {
    if active.is_named(hub_api::NAME) {
        *activation = None;
    }
    Ok(Progress::Done)
}
