//! Boot account identities and trusted Login construction.
use crate::system::control::serve::{Fail, unit::Control};
use crate::system::identity::client::install::Roster;
use super::names::{Names, Registration};
use crate::system::operator::client::Tree;
use env::wire::Span as _;
use env::{PieToken, Wait, pie, unit};
use runtime::schedule::{Progress, Res, ResMut};
use protocol::system::control::{self as control_call, Object, account as call};
use protocol::system::identity::Subject;
use protocol::system::operator::Permit;
use runtime::core::res::pie::{HolePie, reserve};
pub const ACCOUNT: &str = "anran";
pub struct Accounts {
    pub entry: PieToken,
    subject: Option<Subject>,
    image: Option<&'static [u8]>,
}
impl Accounts {
    pub fn new(catalog: crate::boot::Catalog<'static>) -> Result<Self, &'static str> {
        Ok(Self {
            entry: pie::unseal_hole(call::ENTRY).map_err(|_| "account entry")?,
            subject: None,
            image: catalog.find("cat").map(|entry| entry.elf),
        })
    }
}
pub fn initialize(
    mut accounts: ResMut<Accounts>,
    roster: Res<Roster>,
) -> Result<Progress, &'static str> {
    accounts.subject = Some(roster.derive_subject()?);
    Ok(Progress::Done)
}
pub fn account(
    accounts: Res<Accounts>,
    mut names: ResMut<Names>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    let subject = accounts.subject.ok_or("account identity")?;
    names.register(
        &mut tree,
        Registration {
            name: ACCOUNT.into(),
            object: Object::Principal(subject.principal),
            lifetime: None,
        },
    )?;
    Ok(Progress::Done)
}
pub fn publication(
    accounts: Res<Accounts>,
    mut mounts: ResMut<crate::system::boot::Mounts>,
) -> Result<Progress, &'static str> {
    mounts.0.push(super::publication::Internal {
        road: call::DIR.to_path_buf(),
        entry: accounts.entry,
        access: (Permit::Bound, unit::self_id()),
    });
    Ok(Progress::Done)
}
pub fn receive(
    accounts: Res<Accounts>,
    mut control: ResMut<Control>,
    mut pending: ResMut<super::launch::Pending>,
) -> Result<Progress, Fail> {
    let mut bytes = [0; call::Request::LEN];
    for _ in 0..16 {
        let Ok((n, from)) = HolePie::from_token(accounts.entry).pull(&mut bytes, Wait::POLL) else {
            break;
        };
        let Some((raw, _)) = call::Request::fetch_at(&bytes[..n], 0) else {
            continue;
        };
        if !matches!(reserve(raw.back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == call::BACK)
        {
            continue;
        }
        let back = raw.back;
        let result = (|| {
            if control.task("login") != Some(from) || !control.live(from) {
                return Err(control_call::Fail::Denied);
            }
            let request = call::Request::take(&bytes[..n]).ok_or(control_call::Fail::Bad)?;
            if request.account != ACCOUNT {
                return Err(control_call::Fail::Unknown);
            }
            if control
                .instances
                .iter()
                .any(|item| item.owner == from && item.team.is_some())
            {
                return Err(control_call::Fail::NotReady);
            }
            let subject = accounts.subject.ok_or(control_call::Fail::NotReady)?;
            let host = control
                .task("terminal")
                .filter(|host| control.live(*host))
                .ok_or(control_call::Fail::NotReady)?;
            let bytes = accounts.image.ok_or(control_call::Fail::Unknown)?;
            super::launch::construct(
                &mut control,
                &mut pending,
                super::launch::Build {
                    image: crate::system::loader::Image {
                        bytes,
                        kind: env::ProgramKind::User,
                    },
                    spawn: crate::system::loader::serve::build::Spawn {
                        args: &[host.get()],
                        stack: 0,
                    },
                    delivery: super::launch::Delivery {
                        owner: from,
                        identity: protocol::system::identity::Install::Authorized(subject),
                        back,
                    },
                },
            )?;
            Ok(())
        })();
        if let Err(fail) = result {
            super::launch::reply(back, Err(fail));
        }
    }
    Ok(Progress::Done)
}
pub fn watch(
    accounts: Res<Accounts>,
    mut interests: ResMut<super::watch::Interests>,
) -> Result<Progress, Fail> {
    interests.tokens.push(accounts.entry);
    Ok(Progress::Done)
}
