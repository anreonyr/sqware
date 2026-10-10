use super::living::Living;
use crate::system::control::identity::Roster;
use crate::system::control::identity::{current_authority, validate};
use crate::system::control::unit::Control;
use crate::system::operator::Placement;
use crate::system::operator::management::Tree;
use crate::system::operator::tree::Tile;
use ::schedule::{Progress, Res, ResMut};
use alloc::{string::String, vec::Vec};
use env::pie;
use env::{PieToken, TaskId, Wait};
use ipc::rpc;
use system_api::control::publication as pubcall;
use system_api::control::publication::Call as Publication;
use system_api::control::publication::Frame;
use system_api::control::publication::Object;
use system_api::control::publication::Reply;
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Permit;

pub struct Registration {
    pub name: String,
    pub object: Object,
    pub lifetime: Option<TaskId>,
}

enum Lifetime {
    Permanent,
    Task(TaskId),
    Retired,
}
struct Alias {
    name: String,
    object: Object,
    lifetime: Lifetime,
    entry: PieToken,
    pane: EntryId,
    mount: EntryId,
}
pub struct Names {
    aliases: Vec<Alias>,
}
impl Names {
    pub fn new() -> Self {
        Self {
            aliases: Vec::new(),
        }
    }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = PieToken> + '_ {
        self.aliases.iter().map(|a| a.entry)
    }
    pub(super) fn mount_of(&self, name: &str, object: Object) -> Option<EntryId> {
        self.aliases
            .iter()
            .find(|alias| alias.name == name && alias.object == object)
            .map(|alias| alias.mount)
    }
    pub fn register(
        &mut self,
        tree: &mut Tree,
        registration: Registration,
    ) -> Result<(), &'static str> {
        let name = registration.name.as_str();
        let object = registration.object;
        let lifetime = registration.lifetime;

        if !pubcall::valid_name(name) {
            return Err("identity alias name");
        }
        if self
            .aliases
            .iter()
            .any(|a| a.name == name && a.object.kind() == object.kind() && a.object != object)
        {
            return Err("identity alias conflict");
        }
        if self
            .aliases
            .iter()
            .any(|a| a.name == name && a.object == object)
        {
            return Ok(());
        }
        self.aliases
            .try_reserve(1)
            .map_err(|_| "identity alias capacity")?;
        let road = object.road(name).ok_or("identity alias path")?;
        let pane = tree.mount(&Placement {
            road: (road.parent().ok_or("identity alias parent")?).to_path_buf(),
            tile: Tile {
                pie: env::PieToken::NONE,
                permit: Permit::Public,
                owner: None,
            },
            replace: false,
        })?;
        let entry = match pie::unseal(env::UnsealArgs::hole(pubcall::REF)) {
            Ok(entry) => entry,
            Err(_) => {
                let _ = tree.unmount(pane);
                return Err("identity alias entry");
            }
        };
        let mount = match tree.mount(&Placement {
            road: (&road).to_path_buf(),
            tile: Tile {
                pie: entry,
                permit: Permit::Public,
                owner: Some(env::unit::self_id()),
            },
            replace: false,
        }) {
            Ok(mount) => mount,
            Err(why) => {
                let _ = pie::seal(entry);
                let _ = pie::release(entry, env::ReleaseMode::Revoke);
                let _ = tree.unmount(pane);
                return Err(why);
            }
        };
        self.aliases.push(Alias {
            name: name.into(),
            object,
            lifetime: lifetime.map_or(Lifetime::Permanent, Lifetime::Task),
            entry,
            pane,
            mount,
        });
        Ok(())
    }
    fn remove(&mut self, tree: &mut Tree, at: usize) -> Result<(), &'static str> {
        let a = &self.aliases[at];
        tree.unmount(a.mount)?;
        tree.unmount(a.pane)?;
        let _ = pie::seal(a.entry);
        let _ = pie::release(a.entry, env::ReleaseMode::Revoke);
        self.aliases.remove(at);
        Ok(())
    }
}
pub enum AliasRequest {
    Candidate { task: TaskId, object: Object },
    Install(Registration),
}
pub struct Registrations {
    pub requests: Vec<AliasRequest>,
    pub seen: u64,
    pub dirty: bool,
}
pub(crate) fn changes(
    epoch: Res<crate::system::identity::revision::Epoch>,
    changed: Res<crate::system::identity::revision::Changed>,
    mut pending: ResMut<Registrations>,
) -> Result<Progress, &'static str> {
    // Clear before observing the epoch so a later mutation leaves the bell armed.
    match changed.0.hush() {
        Ok(()) => {}
        Err(error) if error.source.is_busy() => {}
        Err(_) => return Err("identity change bell"),
    }
    let revision = epoch.0.load(core::sync::atomic::Ordering::Acquire);
    pending.dirty |= pending.seen != revision;
    pending.seen = revision;
    Ok(Progress::Done)
}
pub(crate) fn expired(
    living: Res<Living>,
    roster: Res<Roster>,
    mut names: ResMut<Names>,
) -> Result<Progress, &'static str> {
    let authority = current_authority(&roster);
    // Mark aliases before tree mutations; no Identity request is made during removal.
    for alias in &mut names.aliases {
        if Some(alias.object.authority()) != authority
            || matches!(alias.lifetime, Lifetime::Task(task) if !living.contains(task))
        {
            alias.lifetime = Lifetime::Retired;
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn retire(
    mut names: ResMut<Names>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    let mut at = 0;
    while at < names.aliases.len() {
        if matches!(names.aliases[at].lifetime, Lifetime::Retired) {
            names.remove(&mut tree, at)?;
        } else {
            at += 1;
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn prepare(
    control: Res<Control>,
    roster: Res<Roster>,
    mut pending: ResMut<Registrations>,
) -> Result<Progress, &'static str> {
    use crate::system::control::unit::table::{Slot, State};
    pending.requests.clear();
    if !pending.dirty {
        return Ok(Progress::Done);
    }
    pending.dirty = false;
    if current_authority(&roster).is_none() {
        return Ok(Progress::Done);
    }
    for row in control.living() {
        if !row.named
            || !matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready | State::Debarked
            )
        {
            continue;
        }
        let Slot::Live { task, .. } = row.slot else {
            continue;
        };
        if env::unit::join_task(task, Wait::POLL).unwrap_or(true) {
            continue;
        }
        if let Some(binding) = crate::system::control::identity::binding(&roster, task)
            .map_err(|_| "alias identity query")?
        {
            pending
                .requests
                .try_reserve(1)
                .map_err(|_| "alias capacity")?;
            pending.requests.push(AliasRequest::Candidate {
                task,
                object: Object::Principal(binding.origin.principal),
            });
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn select(
    control: Res<Control>,
    names: Res<Names>,
    mut pending: ResMut<Registrations>,
) -> Result<Progress, &'static str> {
    let candidates = core::mem::take(&mut pending.requests);
    for request in candidates {
        let AliasRequest::Candidate { task, object } = request else {
            pending
                .requests
                .try_reserve(1)
                .map_err(|_| "alias capacity")?;
            pending.requests.push(request);
            continue;
        };
        let Some(row) = control.find_named_task(task) else {
            continue;
        };
        for name in core::iter::once(row.name.as_str()) {
            if !pubcall::valid_name(name) {
                return Err("deployment alias name");
            }
            if names
                .aliases
                .iter()
                .any(|alias| alias.name == name && alias.object == object)
            {
                continue;
            }
            pending
                .requests
                .try_reserve(1)
                .map_err(|_| "alias capacity")?;
            pending.requests.push(AliasRequest::Install(Registration {
                name: name.into(),
                object,
                lifetime: Some(task),
            }));
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn verify(
    roster: Res<Roster>,
    pending: Res<Registrations>,
) -> Result<Progress, &'static str> {
    for request in &pending.requests {
        let AliasRequest::Install(registration) = request else {
            return Err("alias selection");
        };
        validate(&roster, registration.object).map_err(|_| "identity alias source")?;
    }
    Ok(Progress::Done)
}
pub(crate) fn install(
    mut pending: ResMut<Registrations>,
    mut names: ResMut<Names>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    if tree.host().is_none() {
        return Ok(Progress::Done);
    }
    for request in pending.requests.drain(..) {
        let AliasRequest::Install(registration) = request else {
            return Err("alias selection");
        };
        names.register(&mut tree, registration)?;
    }
    Ok(Progress::Done)
}
pub(crate) fn receive(roster: Res<Roster>, names: Res<Names>) -> Result<Progress, &'static str> {
    let roster = &*roster;

    let mut bytes = [0; Frame::LEN];
    // Ref answers read only the verified index and never wait for Operator.
    for alias in &names.aliases {
        let receiver = rpc::request::Receiver::<Publication>::from_raw(
            alias.entry,
            Publication::BACK,
            Publication::back,
        );
        loop {
            let incoming = match receiver.receive(&mut bytes, Wait::POLL) {
                Ok(incoming) => incoming,
                Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => break,
                Err(_) => continue,
            };
            let frame = incoming.request;
            let accepted = frame.op == pubcall::RESOLVE
                && frame.name == alias.name
                && frame.kind == 10 + alias.object.kind()
                && Some(alias.object.authority()) == current_authority(roster);
            let _ = incoming.reply.send(if accepted {
                Reply::object(alias.object)
            } else {
                Reply::fail(Fail::Unjudged)
            });
        }
    }
    Ok(Progress::Done)
}
