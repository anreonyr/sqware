//! Boot account identities and trusted Login construction.
use crate::system::app::Fault as Fail;
use crate::system::control::identity::Roster;
use crate::system::control::unit::Control;
use crate::system::publication::{Names, Registration};

use crate::system::operator::management::Tree;
use ::schedule::{Progress, Res, ResMut};
use env::{PieToken, Wait, pie, unit};
use ipc::rpc;
use system_api::control as control_call;
use system_api::control::Object;
use system_api::control::account as call;
use system_api::control::account::Call as Account;
use system_api::identity::Subject;
use system_api::operator::Permit;
use wire::Message;
pub(crate) struct Configuration {
    pub name: &'static str,
    pub image: Option<&'static [u8]>,
}
pub struct Accounts {
    pub entry: PieToken,
    subject: Option<Subject>,
    image: Option<&'static [u8]>,
    name: &'static str,
}
impl Accounts {
    pub fn new(config: Configuration) -> Result<Self, &'static str> {
        Ok(Self {
            entry: pie::unseal_hole(call::ENTRY).map_err(|_| "account entry")?,
            subject: None,
            image: config.image,
            name: config.name,
        })
    }
}
pub(crate) fn initialize(
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
            name: accounts.name.into(),
            object: Object::Principal(subject.principal),
            lifetime: None,
        },
    )?;
    Ok(Progress::Done)
}
pub fn publication(
    accounts: Res<Accounts>,
    mut mounts: ResMut<crate::system::publication::Mounts>,
) -> Result<Progress, &'static str> {
    mounts.0.push(crate::system::publication::Internal {
        road: call::DIR.to_path_buf(),
        entry: accounts.entry,
        access: (Permit::Bound, unit::self_id()),
    });
    Ok(Progress::Done)
}
pub fn receive(
    accounts: Res<Accounts>,
    mut control: ResMut<Control>,
    mut pending: ResMut<crate::system::launch::Pending>,
) -> Result<Progress, Fail> {
    let mut bytes = call::Request::EMPTY;
    let receiver =
        rpc::request::Receiver::<Account>::from_raw(accounts.entry, Account::BACK, Account::back);
    for _ in 0..16 {
        let incoming = match receiver.receive(&mut bytes, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => break,
            Err(_) => continue,
        };
        let from = incoming.from;
        let (request, exact) = incoming.request;
        let mut back = Some(incoming.reply);
        let result = (|| {
            if control.task("login") != Some(from) || !control.live(from) {
                return Err(control_call::Fail::Denied);
            }
            if !exact || !system_api::operator::name::valid(&request.account) {
                return Err(control_call::Fail::Bad);
            }
            if request.account != accounts.name {
                return Err(control_call::Fail::Unknown);
            }
            if control.owns_team_instance(from) {
                return Err(control_call::Fail::NotReady);
            }
            let subject = accounts.subject.ok_or(control_call::Fail::NotReady)?;
            let host = control
                .task("terminal")
                .filter(|host| control.live(*host))
                .ok_or(control_call::Fail::NotReady)?;
            let bytes = accounts.image.ok_or(control_call::Fail::Unknown)?;
            match crate::system::launch::construct(
                &mut control,
                &mut pending,
                crate::system::launch::Build {
                    image: crate::system::loader::Image {
                        bytes,
                        kind: env::ProgramKind::User,
                    },
                    spawn: crate::system::loader::Spawn {
                        args: &[host.get()],
                        stack: 0,
                    },
                    delivery: crate::system::launch::Delivery {
                        owner: from,
                        identity: system_api::identity::Install::Authorized(subject),
                        back: back.take().ok_or(control_call::Fail::Bad)?,
                    },
                },
            ) {
                Ok(_) => {}
                Err((fail, back_reply)) => {
                    crate::system::launch::reply(back_reply, Err(fail));
                    return Ok(());
                }
            }
            Ok(())
        })();
        if let Err(fail) = result {
            if let Some(back) = back.take() {
                crate::system::launch::reply(back, Err(fail.into()));
            }
        }
    }
    Ok(Progress::Done)
}
pub fn watch(
    accounts: Res<Accounts>,
    mut interests: ResMut<crate::system::app::wait::Interests>,
) -> Result<Progress, Fail> {
    interests.tokens.push(accounts.entry);
    Ok(Progress::Done)
}

pub(crate) fn install(
    resources: &mut ::schedule::Resources<'static>,
    config: Configuration,
) -> Result<(), &'static str> {
    resources
        .insert(Accounts::new(config)?)
        .map_err(|_| "account resource capacity")
}

pub(crate) fn startup() -> Result<::schedule::Plan<&'static str>, ::schedule::BuildError> {
    let mut plan = ::schedule::Schedule::sequence();
    plan.system("initialize", initialize)?;
    plan.system("identity", account)?;
    plan.system("publication", publication)?;
    plan.build()
}

pub(crate) fn entry(resources: &::schedule::Resources<'_>) -> Result<PieToken, &'static str> {
    Ok(resources
        .read::<Accounts>()
        .map_err(|_| "Account service not installed")?
        .entry)
}
