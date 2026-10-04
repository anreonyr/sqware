//! Boot account identities and trusted Login construction.
use super::{resource::Resources, unit::Control};
use crate::system::control::core::unit::State;
use crate::system::identity::serve::{
    install::Roster,
    names::{Names, Registration},
};
use crate::system::operator::serve::install::Tree;
use alloc::vec::Vec;
use env::wire::Span as _;
use env::{PieToken, TaskId, Wait, pie, unit};
use protocol::common::schedule::{Progress, Res, ResMut};
use protocol::system::control::{self as control_call, Object, account as call};
use protocol::system::identity::Subject;
use protocol::system::operator::Permit;
use runtime::core::res::pie::{HolePie, reserve};
pub const ACCOUNT: &str = "anran";
pub struct Accounts {
    pub entry: PieToken,
    subject: Option<Subject>,
    image: Option<&'static [u8]>,
    replies: Vec<(TaskId, PieToken)>,
}
impl Accounts {
    pub fn new(catalog: crate::boot::Catalog<'static>) -> Result<Self, &'static str> {
        Ok(Self {
            entry: pie::unseal_hole(call::ENTRY).map_err(|_| "account entry")?,
            subject: None,
            image: catalog.find("cat").map(|entry| entry.elf),
            replies: Vec::new(),
        })
    }
}
pub fn initialize(
    mut accounts: ResMut<Accounts>,
    roster: Res<Roster>,
) -> Result<Progress, &'static str> {
    accounts.subject = Some(roster.user()?);
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
fn reply(
    back: PieToken,
    result: Result<protocol::system::loader::Built, control_call::Fail>,
) -> bool {
    let value = match result {
        Ok(built) => protocol::system::loader::frame::Said {
            status: control_call::frame::OK,
            task: built.task,
            team: built.team.get() as u64,
        },
        Err(fail) => protocol::system::loader::frame::Said {
            status: control_call::frame::fail_to_code(Some(fail)),
            task: TaskId::new(0),
            team: 0,
        },
    };
    let mut bytes = [0; protocol::system::loader::frame::Said::LEN];
    let sent = value.store_at(&mut bytes, 0).is_some_and(|n| {
        HolePie::from_token(back)
            .push(&bytes[..n], Wait::POLL)
            .is_ok()
    });
    let _ = pie::release(back);
    sent
}
pub fn receive(
    mut accounts: ResMut<Accounts>,
    mut control: ResMut<Control>,
    roster: Res<Roster>,
) -> Result<Progress, super::Fail> {
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
            accounts
                .replies
                .try_reserve(1)
                .map_err(|_| control_call::Fail::Full)?;
            let subject = accounts.subject.ok_or(control_call::Fail::NotReady)?;
            let host = control
                .task("terminal")
                .filter(|host| control.live(*host))
                .ok_or(control_call::Fail::NotReady)?;
            let bytes = accounts.image.ok_or(control_call::Fail::Unknown)?;
            let built = crate::system::loader::serve::build::construct_image(
                &mut control,
                &roster,
                crate::system::loader::serve::build::Build {
                    image: crate::system::loader::Image {
                        bytes,
                        kind: env::ProgramKind::User,
                    },
                    spawn: crate::system::loader::serve::build::Spawn {
                        owner: from,
                        args: &[host.get()],
                        stack: 0,
                    },
                    subject: Some(subject),
                },
            )?;
            accounts.replies.push((built.task, back));
            Ok(())
        })();
        if let Err(fail) = result {
            reply(back, Err(fail));
        }
    }
    Ok(Progress::Done)
}
pub fn completed(
    mut accounts: ResMut<Accounts>,
    mut control: ResMut<Control>,
    resources: Res<Resources>,
) -> Result<Progress, super::Fail> {
    let mut index = 0;
    while index < accounts.replies.len() {
        let (task, back) = accounts.replies[index];
        let item = control.instances.iter_mut().find(|item| item.task == task);
        match item {
            Some(item)
                if item.state == State::Debarked && resources.runtime_road(task).is_some() =>
            {
                let built = protocol::system::loader::Built {
                    task,
                    team: item.team.ok_or(super::Fail::Room)?,
                };
                if !reply(back, Ok(built)) {
                    item.state = State::Stopping;
                }
            }
            Some(item) if matches!(item.state, State::Dead | State::Stopping) => {
                reply(back, Err(control_call::Fail::NotReady));
            }
            None => {
                reply(back, Err(control_call::Fail::NotReady));
            }
            _ => {
                index += 1;
                continue;
            }
        }
        accounts.replies.remove(index);
    }
    Ok(Progress::Done)
}
pub fn watch(
    accounts: Res<Accounts>,
    mut interests: ResMut<super::watch::Interests>,
) -> Result<Progress, super::Fail> {
    interests.tokens.push(accounts.entry);
    Ok(Progress::Done)
}
