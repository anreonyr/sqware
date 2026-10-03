//! Control owns this private installation face; only the live Hub can ask through it.
use env::{Access, PieToken, Policy, TaskId, Wait};
use protocol::communication::hand::Sender;
use protocol::communication::session::establish;
use protocol::service::hub::{self, activation::{self, Activate}, frame::Said};
use protocol::system::identity::CoalitionId;
use runtime::core::res::port;
use runtime::env::{mail::{self, HolePie}, unit};

use crate::system::control::{BOOT_MS, Control};
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
            // **一趟里的每一枚都要落**：有一枚不成 ⇒ 整趟答 `DENIED`（起手那一侧据此当场收手）
            let status = if allowed
                && (0..ask.len()).all(|i| {
                    ask.coalition(i)
                        .is_some_and(|c| control.roster.activate(ask.task, c).is_ok())
                })
            {
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
/// **一趟报一批**（见 [`activation::Activate`]）：枚数写进帧里，`BACK` 那一条回话只有一个状态。
pub fn activate(task: TaskId, coalitions: &[CoalitionId]) -> Result<(), ()> {
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
    let frame = Activate::of(task, coalitions, seed).ok_or(())?;
    let mut request = Sender::<Activate>::from_token(entry);
    request.send_within(frame, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())?;
    let mut bytes = [0; Said::LEN];
    let (n, from) = HolePie::from_token(back).pull(&mut bytes, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())?;
    let said = Said::fetch_at(&bytes[..n], 0).map(|one| one.0).ok_or(())?;
    (from == sire && said.status == hub::OK).then_some(()).ok_or(())
}

pub(crate) fn publication(
    _program: &crate::unit::UnitFile, from: env::TaskId,
    target: &protocol::system::control::publication::Target,
    mark: env::Mark, requested: protocol::system::operator::Permit,
    machine: &crate::system::common::machine::Machine,
    roster: &crate::system::identity::bridge::Roster,
) -> Result<protocol::common::path::PathBuf, protocol::system::operator::Fail> {
    use protocol::common::path::Path;
    use protocol::system::control::publication::{Target, Scope, Object};
    use protocol::system::operator::{Permit, Fail};
    use protocol::system::identity::Selector;
    use crate::system::identity::bridge::{binding, validate};
    match target {
            Target::Service {
                scope: Scope::Hub,
                group,
                name,
            } => {
                if !group.is_empty() || requested != Permit::Public {
                    return Err(Fail::Denied);
                }
                let grant = protocol::service::hub::Grant::ALL
                    .iter()
                    .find(|g| g.name() == name && g.mark() == mark)
                    .ok_or(Fail::Denied)?;
                Path::new("svc/hub").try_join(grant.name()).ok_or(Fail::Denied)
            }
            Target::Service {
                scope: Scope::Device,
                group,
                name,
            } => {
                if mark != protocol::service::hub::Grant::Claim.mark() {
                    return Err(Fail::Denied);
                }
                let Permit::Identity(Selector::MemberOf(c)) = requested else {
                    return Err(Fail::Denied);
                };
                let subject = binding(roster, from)?.ok_or(Fail::Denied)?.current;
                if !subject.coalitions.contains(c) {
                    return Err(Fail::Denied);
                }
                let valid = (group == protocol::service::hub::BOOT
                    && [protocol::service::hub::DTB, protocol::service::hub::IRQ]
                        .contains(&name.as_str()))
                    || machine.devices().is_some_and(|devices| {
                        devices
                            .iter()
                            .any(|d| d.class.as_str() == group && d.name.as_str() == name)
                    });
                if !valid {
                    return Err(Fail::Denied);
                }
                validate(roster, Object::Coalition(c))?;
                Path::new("dev").try_join(group).and_then(|p| p.try_join(name)).ok_or(Fail::Denied)
            }
        _ => Err(Fail::Denied),
    }
}
