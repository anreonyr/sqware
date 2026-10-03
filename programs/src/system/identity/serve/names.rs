use crate::system::operator::serve::plate::Placement;
use crate::system::operator::core::Tile;
use protocol::common::schedule::{Progress, Res, ResMut};
use crate::system::control::serve::{living::Living, unit::Control};
use crate::system::identity::serve::install::Roster;
use crate::system::identity::serve::query::{current_authority, validate};
use crate::system::operator::serve::install::Tree;
use alloc::{string::String, vec::Vec};
use env::wire::Span as _;
use env::{PieToken, TaskId, Wait};
use protocol::system::control::publication::{self as pubcall, Frame, Object, Reply};
use protocol::system::operator::{EntryId, Fail, Permit};
use runtime::env::mail::{self, HolePie};
pub struct Registration { pub name: String, pub object: Object, pub lifetime: Option<TaskId> }

enum Lifetime { Permanent, Task(TaskId), Retired }
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
    pub fn register(&mut self, tree: &mut Tree, registration: Registration) -> Result<(), &'static str> {
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
        let pane = tree.mount(&Placement { road: (road.parent().ok_or("identity alias parent")?).to_path_buf(), tile: Tile { pie: env::PieToken::NONE, permit: Permit::Public, owner: None }, replace: false })?;
        let entry = match mail::unseal_hole(pubcall::REF) {
            Ok(entry) => entry,
            Err(_) => {
                let _ = tree.unmount(pane);
                return Err("identity alias entry");
            }
        };
        let mount = match tree.mount(&Placement { road: (&road).to_path_buf(), tile: Tile { pie: entry, permit: Permit::Public, owner: Some(runtime::env::unit::self_id()) }, replace: false }) {
            Ok(mount) => mount,
            Err(why) => {
                let _ = mail::seal(entry);
                let _ = mail::release(entry);
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
        let _ = mail::seal(a.entry);
        let _ = mail::release(a.entry);
        self.aliases.remove(at);
        Ok(())
    }

}
fn valid_back(back: PieToken, from: TaskId) -> bool {
    matches!(mail::reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == pubcall::BACK)
}
fn reply(back: PieToken, reply: Reply) {
    let mut bytes = [0; Reply::LEN];
    if let Some(n) = reply.store_at(&mut bytes, 0) {
        let _ = HolePie::from_token(back).push(&bytes[..n], Wait::POLL);
    }
    let _ = mail::release(back);
}

pub struct Registrations(pub Vec<Registration>);
pub(crate) fn expired(living: Res<Living>, roster: Res<Roster>, mut names: ResMut<Names>) -> Result<Progress, &'static str> {
    let authority = current_authority(&roster);
    // Mark aliases before tree mutations; no Identity request is made during removal.
    for alias in &mut names.aliases {
        if Some(alias.object.authority()) != authority || matches!(alias.lifetime, Lifetime::Task(task) if !living.contains(task)) {
            alias.lifetime = Lifetime::Retired;
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn retire(mut names: ResMut<Names>, mut tree: ResMut<Tree>) -> Result<Progress, &'static str> {
    let mut at = 0;
    while at < names.aliases.len() {
        if matches!(names.aliases[at].lifetime, Lifetime::Retired) { names.remove(&mut tree, at)?; } else { at += 1; }
    }
    Ok(Progress::Done)
}
pub(crate) fn prepare(control: Res<Control>, roster: Res<Roster>, mut pending: ResMut<Registrations>) -> Result<Progress, &'static str> {
    use crate::system::control::core::unit::{Slot, State};
    pending.0.clear();
    if current_authority(&roster).is_none() { return Ok(Progress::Done); }
    for row in control.table.living() {
        if !row.named || !matches!(row.state, State::NeverStarted | State::Starting | State::Ready | State::Debarked) { continue; }
        let Slot::Live { task, .. } = row.slot else { continue; };
        if runtime::env::unit::join(task, Wait::POLL).unwrap_or(true) { continue; }
        if let Some(binding) = super::query::binding(&roster, task).map_err(|_| "alias identity query")? {
            let registration = Registration { name: row.name.clone(), object: Object::Principal(binding.origin.principal), lifetime: Some(task) };
            validate(&roster, registration.object).map_err(|_| "identity alias source")?;
            pending.0.try_reserve(1).map_err(|_| "alias capacity")?;
            pending.0.push(registration);
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn install(mut pending: ResMut<Registrations>, mut names: ResMut<Names>, mut tree: ResMut<Tree>) -> Result<Progress, &'static str> {
    if tree.host().is_none() { return Ok(Progress::Done); }
    for registration in pending.0.drain(..) { names.register(&mut tree, registration)?; }
    Ok(Progress::Done)
}
pub(crate) fn receive(roster: Res<Roster>, names: Res<Names>) -> Result<Progress, &'static str> {
    let roster = &*roster;

    let mut bytes = [0; Frame::LEN];
    // Ref answers read only the verified index and never wait for Operator.
    for alias in &names.aliases {
        while let Ok((n, from)) = HolePie::from_token(alias.entry).pull(&mut bytes, Wait::POLL)
        {
            let Some(frame) = Frame::take(&bytes[..n]) else {
                continue;
            };
            if !valid_back(frame.back, from) {
                continue;
            }
            let accepted = frame.op == pubcall::RESOLVE
                && frame.name == alias.name
                && frame.kind == 10 + alias.object.kind()
                && Some(alias.object.authority()) == current_authority(roster);
            reply(
                frame.back,
                if accepted {
                    Reply::object(alias.object)
                } else {
                    Reply::fail(Fail::Unjudged)
                },
            );
        }
    }
    Ok(Progress::Done)
}
